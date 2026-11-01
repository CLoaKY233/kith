//! `kith request-code`, `request`, `requests`, `approve` and `deny`.

#![expect(
    clippy::wildcard_enum_match_arm,
    reason = "any answer other than the expected ones is a protocol error"
)]

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use kith_ipc::{IpcClient, IpcError, PendingJoin, Request, Response};
use kith_types::NetworkId;

use crate::cli::AdmissionCmd;
use crate::exec::{CliError, Names, connect, fetch_status, hostname};
use crate::render::{printable, write_row};

/// How often a waiting device asks again.
const POLL: Duration = Duration::from_secs(3);

/// How many times to ask before giving up: requests expire on the members after 10 minutes.
const MAX_POLLS: u32 = 200;

pub(crate) async fn run(
    cmd: &AdmissionCmd,
    socket: &Path,
    out: &mut impl Write,
) -> Result<(), CliError> {
    let mut client = connect(socket).await?;
    match cmd {
        AdmissionCmd::RequestCode { network } => {
            let network = names(&mut client).await?.network(network)?;
            match client.request(Request::RequestCode { network }).await? {
                Response::RequestCode { code } => {
                    writeln!(out, "{code}")?;
                    writeln!(
                        out,
                        "on the new device: kith request <code>; then an admin runs kith approve"
                    )?;
                    Ok(())
                }
                other => unexpected(other),
            }
        }
        AdmissionCmd::Request { name, code } => {
            let device_name = name.clone().unwrap_or_else(hostname);
            wait_for_answer(&mut client, code, device_name, out).await
        }
        AdmissionCmd::Requests => {
            let pending = pending(&mut client).await?;
            let names = names(&mut client).await?;
            table(out, &pending, &names)
        }
        AdmissionCmd::Approve { network, device } => {
            let (network, node, name) = find(&mut client, network, device).await?;
            match client
                .request(Request::ApproveJoin { network, node })
                .await?
            {
                Response::JoinApproved { device } => {
                    writeln!(
                        out,
                        "approved {}; it joins as {}",
                        printable(&name),
                        printable(&device)
                    )?;
                    Ok(())
                }
                other => unexpected(other),
            }
        }
        AdmissionCmd::Deny { network, device } => {
            let (network, node, name) = find(&mut client, network, device).await?;
            match client.request(Request::DenyJoin { network, node }).await? {
                Response::JoinRequestDenied => {
                    writeln!(out, "denied {}", printable(&name))?;
                    Ok(())
                }
                other => unexpected(other),
            }
        }
    }
}

async fn wait_for_answer(
    client: &mut IpcClient,
    code: &str,
    device_name: String,
    out: &mut impl Write,
) -> Result<(), CliError> {
    let request = Request::RequestJoin {
        request_code: code.to_owned(),
        device_name,
    };
    let mut polls: u32 = 0;
    let mut network_name = None;
    loop {
        match client.request(request.clone()).await? {
            Response::JoinPending {
                network_name: name, ..
            } => {
                if network_name.is_none() {
                    writeln!(out, "waiting for an admin to approve...")?;
                    out.flush()?;
                    network_name = Some(name);
                }
                polls = polls.saturating_add(1);
                if polls >= MAX_POLLS {
                    return Err(CliError(
                        "no admin answered within 10 minutes; run kith request again".to_owned(),
                    ));
                }
                tokio::time::sleep(POLL).await;
            }
            Response::Joined { network, device } => {
                let network = network_name.unwrap_or_else(|| network.to_string());
                writeln!(
                    out,
                    "joined {} as {}",
                    printable(&network),
                    printable(&device)
                )?;
                return Ok(());
            }
            Response::JoinDenied { .. } => {
                return Err(CliError(
                    "an admin turned this device down; ask them, then run kith request again"
                        .to_owned(),
                ));
            }
            other => return unexpected(other),
        }
    }
}

async fn names(client: &mut IpcClient) -> Result<Names, CliError> {
    let status = fetch_status(client).await?;
    Ok(Names {
        networks: status
            .networks
            .iter()
            .map(|network| (network.id, network.name.clone()))
            .collect(),
        ..Names::default()
    })
}

async fn pending(client: &mut IpcClient) -> Result<Vec<PendingJoin>, CliError> {
    match client.request(Request::PendingJoins).await? {
        Response::PendingJoins(pending) => Ok(pending),
        other => unexpected(other),
    }
}

/// The waiting device in `network` whose requested name is `device`, or whose id is or
/// starts with it.
async fn find(
    client: &mut IpcClient,
    network: &str,
    device: &str,
) -> Result<(NetworkId, kith_types::NodeId, String), CliError> {
    let network = names(client).await?.network(network)?;
    let waiting: Vec<PendingJoin> = pending(client)
        .await?
        .into_iter()
        .filter(|held| held.network == network)
        .filter(|held| {
            held.name == device || (device.len() >= 8 && held.node.to_string().starts_with(device))
        })
        .collect();
    match waiting.as_slice() {
        [held] => Ok((network, held.node, held.name.clone())),
        [] => Err(CliError(format!(
            "no device named `{device}` is waiting; see kith requests"
        ))),
        _ => Err(CliError(format!(
            "several waiting devices are named `{device}`; use its id from kith requests"
        ))),
    }
}

fn table(out: &mut impl Write, pending: &[PendingJoin], names: &Names) -> Result<(), CliError> {
    if pending.is_empty() {
        writeln!(out, "no devices are waiting to join")?;
        return Ok(());
    }
    let rows: Vec<[String; 5]> = pending
        .iter()
        .map(|held| {
            let network = names
                .networks
                .iter()
                .find(|(id, _)| *id == held.network)
                .map_or_else(|| held.network.to_string(), |(_, name)| name.clone());
            [
                network,
                held.name.clone(),
                held.node.to_string(),
                held.from.clone().unwrap_or_else(|| "-".to_owned()),
                format!("{}m{}s ago", held.age_secs / 60, held.age_secs % 60),
            ]
        })
        .collect();
    let header = ["NETWORK", "NAME", "ID", "FROM", "ASKED"];
    let mut widths = header.map(str::len);
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    write_row(out, "", &header, &widths)?;
    for row in &rows {
        write_row(out, "", row, &widths)?;
    }
    Ok(())
}

fn unexpected<T>(response: Response) -> Result<T, CliError> {
    drop(response);
    Err(IpcError::Unexpected.into())
}
