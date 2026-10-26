//! macOS: `route` for routes, `/etc/resolver` files for split DNS, and a pf
//! anchor under `com.apple/`, which the stock `/etc/pf.conf` already references.

use std::path::Path;

use kith_types::IpPrefix;

use crate::{
    Cmd, Dns, Error, Interface, Plan, System,
    sys::{Errors, Sys, field, lines},
};

const RESOLVER_DIR: &str = "/etc/resolver";
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

fn host(verb: &str, addr: core::net::IpAddr) -> Cmd {
    let cmd = Cmd::new("route").args(["-n", verb]);
    let cmd = if addr.is_ipv6() {
        cmd.arg("-inet6")
    } else {
        cmd
    };
    cmd.arg("-host").arg(addr)
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
        sys.run(net("add", v6, prefix, dev))?;
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
            let (gateway, _) = default_route(sys, peer.is_ipv6())?;
            sys.run(host("delete", peer).tolerate(NOT_IN_TABLE))?;
            sys.run(host("add", peer).arg(gateway))?;
        }
        for (v6, half) in HALVES {
            sys.run(net("add", v6, half, dev))?;
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

fn apply_dns<S: System>(sys: &mut Sys<S>, dns: &Dns) -> Result<(), Error> {
    if dns.all {
        return Err(Error::Unsupported("all-DNS mode on macOS"));
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

fn revert_dns<S: System>(sys: &mut Sys<S>) -> Result<(), Error> {
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
