//! macOS: `route` for routes, `/etc/resolver` files for split DNS, and a pf
//! anchor under `com.apple/`, which the stock `/etc/pf.conf` already references.

use std::net::IpAddr;
use std::path::Path;

use kith_types::IpPrefix;

use crate::{
    Cmd, Dns, Error, Interface, Plan, System,
    sys::{Errors, Sys, field, lines, nameservers},
};

const RESOLVER_DIR: &str = "/etc/resolver";
/// Each network service's DNS servers before kith overrode them, `name<TAB>servers`.
const DNS_SERVICES: &str = "dns-services";
/// The resolvers macOS used before kith overrode DNS, one per line.
const DNS_UPSTREAMS: &str = "dns-upstreams";
const RESOLVER_MARKER: &str = "# Managed by kith; removed when kith stops.\n";
const ANCHOR: &str = "com.apple/kith";
const NOT_IN_TABLE: &str = "not in table";
const HALVES: [(bool, &str); 4] = [
    (false, "0.0.0.0/1"),
    (false, "128.0.0.0/1"),
    (true, "::/1"),
    (true, "8000::/1"),
];

fn net(verb: &str, v6: bool, dest: impl core::fmt::Display, dev: &Interface) -> Cmd {
    Cmd::new("route")
        .args(["-n", verb, if v6 { "-inet6" } else { "-net" }])
        .arg(dest)
        .arg("-interface")
        .arg(dev)
}

fn host(verb: &str, addr: IpAddr) -> Cmd {
    let cmd = Cmd::new("route").args(["-n", verb]);
    let cmd = if addr.is_ipv6() {
        cmd.arg("-inet6")
    } else {
        cmd
    };
    cmd.arg("-host").arg(addr)
}

/// `-ifa <source>` for a route to `dest`: the source inside it, else the first of its family.
fn source_for(sources: &[IpAddr], dest: &IpPrefix) -> Vec<String> {
    let v6 = matches!(dest, IpPrefix::V6(_));
    sources
        .iter()
        .find(|addr| dest.contains(**addr))
        .or_else(|| sources.iter().find(|addr| addr.is_ipv6() == v6))
        .map(|addr| vec!["-ifa".to_owned(), addr.to_string()])
        .unwrap_or_default()
}

/// How `peer` is reached now, as `route add` arguments: its gateway, or the interface of a
/// directly connected network such as a VM bridge or the local LAN.
fn current_route<S: System>(sys: &mut Sys<S>, peer: IpAddr) -> Result<Vec<String>, Error> {
    let cmd = Cmd::new("route").args(["-n", "get"]);
    let cmd = if peer.is_ipv6() {
        cmd.arg("-inet6")
    } else {
        cmd
    }
    .arg(peer);
    let line = cmd.to_string();
    let out = sys.exec(&cmd)?.stdout;
    let gateway = field(&out, "gateway").filter(|gw| gw.parse::<IpAddr>().is_ok());
    match (gateway, field(&out, "interface")) {
        (Some(gw), _) => Ok(vec![gw.to_owned()]),
        (None, Some(dev)) => Ok(vec![
            "-interface".to_owned(),
            Interface::new(dev)?.as_str().to_owned(),
        ]),
        (None, None) => Err(Error::Parse {
            command: line,
            output: out,
        }),
    }
}

/// The `(gateway, interface)` of the default route.
fn default_route<S: System>(sys: &mut Sys<S>, v6: bool) -> Result<(String, String), Error> {
    let cmd = Cmd::new("route").args(["-n", "get"]);
    let cmd = if v6 { cmd.arg("-inet6") } else { cmd }.arg("default");
    let line = cmd.to_string();
    let out = sys.exec(&cmd)?.stdout;
    match (field(&out, "gateway"), field(&out, "interface")) {
        (Some(gw), Some(dev)) => Ok((gw.to_owned(), dev.to_owned())),
        _ => Err(Error::Parse {
            command: line,
            output: out,
        }),
    }
}

pub(crate) fn apply<S: System>(sys: &mut Sys<S>, plan: &Plan) -> Result<(), Error> {
    let dev = &plan.interface;
    // Delete before add: utun numbering changes between runs.
    for prefix in &plan.routes {
        let v6 = matches!(prefix, IpPrefix::V6(_));
        sys.run(net("delete", v6, prefix, dev).tolerate(NOT_IN_TABLE))?;
        sys.run(net("add", v6, prefix, dev).args(source_for(&plan.sources, prefix)))?;
    }
    if let Some(exit) = &plan.exit_node {
        // Remove our halves first so `route get default` sees the real gateway.
        for (v6, half) in HALVES {
            sys.run(net("delete", v6, half, dev).tolerate(NOT_IN_TABLE))?;
        }
        let state = sys.state("exit-hosts");
        let mut hosts = sys.read_list(&state)?;
        hosts.extend(exit.peer_endpoints.iter().map(ToString::to_string));
        hosts.sort();
        hosts.dedup();
        sys.system.write(&state, &lines(&hosts))?;
        for &peer in &exit.peer_endpoints {
            sys.run(host("delete", peer).tolerate(NOT_IN_TABLE))?;
            let via = current_route(sys, peer)?;
            sys.run(host("add", peer).args(via))?;
        }
        for (v6, half) in HALVES {
            let prefix: IpPrefix = half.parse().map_err(|_| Error::Parse {
                command: "exit route".to_owned(),
                output: half.to_owned(),
            })?;
            sys.run(net("add", v6, half, dev).args(source_for(&plan.sources, &prefix)))?;
        }
    }
    if let Some(dns) = &plan.dns {
        apply_dns(sys, dns)?;
    }
    if plan.forwarding {
        sys.enable_sysctls(&["net.inet.ip.forwarding", "net.inet6.ip6.forwarding"])?;
    }
    if let Some(nat) = plan.nat.as_ref().filter(|n| !n.sources.is_empty()) {
        let wan = Interface::new(&default_route(sys, false)?.1)?;
        let rules = lines(nat.sources.iter().map(|src| {
            let family = if matches!(src, IpPrefix::V6(_)) {
                "inet6"
            } else {
                "inet"
            };
            format!("nat on {wan} {family} from {src} to any -> ({wan})")
        }));
        sys.run(
            Cmd::new("pfctl")
                .args(["-a", ANCHOR, "-f", "-"])
                .stdin(rules),
        )?;
        let token = sys.state("pf-token");
        if sys.system.read(&token)?.is_none() {
            let out = sys.exec(&Cmd::new("pfctl").arg("-E"))?;
            let text = format!("{}{}", out.stdout, out.stderr);
            let value = text
                .split("Token :")
                .nth(1)
                .and_then(|rest| rest.split_whitespace().next())
                .filter(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()))
                .ok_or(Error::Parse {
                    command: "pfctl -E".to_owned(),
                    output: text.clone(),
                })?;
            sys.system.write(&token, value)?;
        }
    }
    Ok(())
}

pub(crate) fn revert<S: System>(sys: &mut Sys<S>, plan: &Plan, errors: &mut Errors) {
    let dev = &plan.interface;
    errors.keep(sys.run(Cmd::new("pfctl").args(["-a", ANCHOR, "-F", "all"])));
    errors.keep(revert_pf_token(sys));
    errors.keep(sys.restore_sysctls());
    errors.keep(revert_dns(sys));
    for (v6, half) in HALVES {
        errors.keep(sys.run(net("delete", v6, half, dev).tolerate(NOT_IN_TABLE)));
    }
    errors.keep(revert_hosts(sys));
    for prefix in &plan.routes {
        let v6 = matches!(prefix, IpPrefix::V6(_));
        errors.keep(sys.run(net("delete", v6, prefix, dev).tolerate(NOT_IN_TABLE)));
    }
}

fn revert_pf_token<S: System>(sys: &mut Sys<S>) -> Result<(), Error> {
    let path = sys.state("pf-token");
    if let Some(token) = sys.system.read(&path)? {
        sys.run(Cmd::new("pfctl").arg("-X").arg(token.trim()))?;
        sys.system.remove(&path)?;
    }
    Ok(())
}

fn revert_hosts<S: System>(sys: &mut Sys<S>) -> Result<(), Error> {
    let path = sys.state("exit-hosts");
    for line in sys.read_list(&path)? {
        let addr = line.parse().map_err(|_| Error::Parse {
            command: "exit-hosts".to_owned(),
            output: line,
        })?;
        sys.run(host("delete", addr).tolerate(NOT_IN_TABLE))?;
    }
    sys.system.remove(&path)
}

/// Split DNS through `/etc/resolver` files; all-DNS mode also points every network service
/// at the mesh resolver, saving each service's servers first so stopping restores them.
fn apply_dns<S: System>(sys: &mut Sys<S>, dns: &Dns) -> Result<(), Error> {
    if dns.all {
        override_dns(sys, dns)?;
    }
    for domain in &dns.domains {
        let path = Path::new(RESOLVER_DIR).join(domain.as_str());
        if sys
            .system
            .read(&path)?
            .is_some_and(|t| !t.starts_with(RESOLVER_MARKER))
        {
            return Err(Error::Conflict(path));
        }
        sys.system.write(
            &path,
            &format!("{RESOLVER_MARKER}nameserver {}\n", dns.resolver),
        )?;
    }
    Ok(())
}

fn override_dns<S: System>(sys: &mut Sys<S>, dns: &Dns) -> Result<(), Error> {
    let saved_path = sys.state(DNS_SERVICES);
    if sys.system.read(&saved_path)?.is_none() {
        let upstreams = system_resolvers(sys)?;
        sys.system
            .write(&sys.state(DNS_UPSTREAMS), &lines(upstreams))?;
        let mut saved = Vec::new();
        for service in network_services(sys)? {
            let servers = service_dns(sys, &service)?;
            let servers = if servers.is_empty() {
                "Empty".to_owned()
            } else {
                servers.join(" ")
            };
            saved.push(format!("{service}\t{servers}"));
        }
        sys.system.write(&saved_path, &lines(saved))?;
    }
    for line in sys.read_list(&saved_path)? {
        let Some((service, _)) = line.split_once('\t') else {
            continue;
        };
        sys.run(
            Cmd::new("networksetup")
                .arg("-setdnsservers")
                .arg(service)
                .arg(dns.resolver),
        )?;
    }
    Ok(())
}

/// Enabled network services whose names kith can pass to `networksetup`.
fn network_services<S: System>(sys: &mut Sys<S>) -> Result<Vec<String>, Error> {
    let out = sys
        .exec(&Cmd::new("networksetup").arg("-listallnetworkservices"))?
        .stdout;
    Ok(out
        .lines()
        .skip(1)
        .filter(|name| !name.starts_with('*'))
        .filter(|name| {
            let ok = crate::validate::valid_service(name);
            if !ok {
                tracing::warn!(
                    service = name,
                    "skipping a network service kith cannot name safely"
                );
            }
            ok
        })
        .map(str::to_owned)
        .collect())
}

/// A service's manually set DNS servers; empty when it uses the ones DHCP provides.
fn service_dns<S: System>(sys: &mut Sys<S>, service: &str) -> Result<Vec<String>, Error> {
    let out = sys
        .exec(&Cmd::new("networksetup").arg("-getdnsservers").arg(service))?
        .stdout;
    Ok(out
        .lines()
        .filter(|line| line.parse::<IpAddr>().is_ok())
        .map(str::to_owned)
        .collect())
}

fn restore_dns<S: System>(sys: &mut Sys<S>) -> Result<(), Error> {
    let saved_path = sys.state(DNS_SERVICES);
    for line in sys.read_list(&saved_path)? {
        let Some((service, servers)) = line.split_once('\t') else {
            continue;
        };
        sys.run(
            Cmd::new("networksetup")
                .arg("-setdnsservers")
                .arg(service)
                .args(servers.split(' ')),
        )?;
    }
    sys.system.remove(&sys.state(DNS_UPSTREAMS))?;
    sys.system.remove(&saved_path)
}

/// The resolvers macOS uses for names outside any `/etc/resolver` domain, skipping kith's
/// own; while kith overrides DNS, the ones it saved before doing so.
pub(crate) fn system_resolvers<S: System>(sys: &mut Sys<S>) -> Result<Vec<IpAddr>, Error> {
    let out = sys.exec(&Cmd::new("scutil").arg("--dns"))?.stdout;
    let primary = out
        .split("\nresolver #")
        .skip(1)
        .find(|block| !block.lines().any(|l| l.trim_start().starts_with("domain")))
        .unwrap_or_default();
    let found = nameservers(primary.lines().filter_map(|l| {
        l.trim_start()
            .strip_prefix("nameserver[")
            .and_then(|rest| rest.split_once(':'))
            .map(|(_, ip)| ip.trim())
    }));
    if !found.is_empty() {
        return Ok(found);
    }
    let saved = sys.read_list(&sys.state(DNS_UPSTREAMS))?;
    Ok(nameservers(saved.iter().map(String::as_str)))
}

fn revert_dns<S: System>(sys: &mut Sys<S>) -> Result<(), Error> {
    let mut errors = Errors::default();
    errors.keep(restore_dns(sys));
    errors.keep(revert_resolver_files(sys));
    errors.finish()
}

fn revert_resolver_files<S: System>(sys: &mut Sys<S>) -> Result<(), Error> {
    for path in sys.system.list(Path::new(RESOLVER_DIR))? {
        if sys
            .system
            .read(&path)?
            .is_some_and(|t| t.starts_with(RESOLVER_MARKER))
        {
            sys.system.remove(&path)?;
        }
    }
    if sys.system.list(Path::new(RESOLVER_DIR))?.is_empty() {
        sys.run(Cmd::new("rmdir").arg(RESOLVER_DIR).tolerate("No such file"))?;
    }
    Ok(())
}
