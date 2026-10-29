use std::{
    collections::{BTreeMap, BTreeSet},
    net::IpAddr,
    path::{Path, PathBuf},
};

use core::fmt::Write as _;
use kith_types::IpPrefix;

use proptest::prelude::*;

use crate::{
    Cmd, Dns, Domain, Error, ExitNode, Interface, Nat, Output, Plan, Platform, Sys, System,
    validate::valid_arg,
};

/// The OS state the fake models; `apply` + `revert` must return it unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct World {
    routes: BTreeMap<String, String>,
    rules: Vec<String>,
    sysctl: BTreeMap<String, String>,
    files: BTreeMap<PathBuf, String>,
    nft: Option<String>,
    pf_anchor: Option<String>,
    pf_tokens: BTreeSet<u64>,
    resolved: BTreeMap<String, String>,
    resolvconf: BTreeMap<String, String>,
    programs: BTreeSet<String>,
    /// Docker's `DOCKER-USER` chain by rule handle, when Docker runs.
    docker: Option<BTreeMap<u64, String>>,
    /// macOS network services and their manual DNS servers; `*` marks a disabled one.
    services: Vec<(String, String)>,
    /// What `ip route show table all` or `netstat -rn` prints, by address family.
    route_table: BTreeMap<bool, String>,
}

#[derive(Debug, Default)]
struct Fake {
    world: World,
    log: Vec<String>,
    unknown: Vec<String>,
    /// Fails the command this many commands from now.
    fail_in: Option<usize>,
}

fn fail(cmd: &Cmd, stderr: &str) -> Result<Output, Error> {
    Err(Error::Command {
        command: cmd.to_string(),
        code: Some(2),
        stderr: stderr.to_owned(),
    })
}

#[expect(clippy::unnecessary_wraps, reason = "mirrors `System::run`")]
fn ok(stdout: &str, stderr: &str) -> Result<Output, Error> {
    Ok(Output {
        stdout: stdout.to_owned(),
        stderr: stderr.to_owned(),
    })
}

impl Fake {
    fn linux(&mut self, cmd: &Cmd, args: &[&str], stdin: String) -> Option<Result<Output, Error>> {
        let w = &mut self.world;
        match (cmd.program(), args) {
            ("ip", [fam, "route", "replace", dst, "dev", dev, rest @ ..]) => {
                let table = rest.get(1).unwrap_or(&"main");
                w.routes
                    .insert(format!("{fam} {dst} {table}"), (*dev).to_owned());
            }
            ("ip", [fam, "route", "del", dst, "dev", _]) => {
                if w.routes.remove(&format!("{fam} {dst} main")).is_none() {
                    return Some(fail(cmd, "RTNETLINK answers: No such process"));
                }
            }
            ("ip", [fam, "route", "flush", "table", t]) => {
                w.routes
                    .retain(|k, _| !(k.starts_with(fam) && k.ends_with(&format!(" {t}"))));
            }
            ("ip", ["-4", "route", "show", "default"]) => {
                return Some(ok("default via 192.168.1.1 dev eth0 proto dhcp\n", ""));
            }
            ("ip", [fam, "rule", op, spec @ ..]) => {
                let key = format!("{fam} {}", spec.join(" "));
                match *op {
                    "add" => w.rules.push(key),
                    _ => match w.rules.iter().position(|r| *r == key) {
                        Some(i) => drop(w.rules.remove(i)),
                        None => {
                            return Some(fail(cmd, "RTNETLINK answers: No such file or directory"));
                        }
                    },
                }
            }
            ("sysctl", ["-n", key]) => return Some(ok(&format!("{}\n", w.sysctl[*key]), "")),
            ("sysctl", ["-w", kv]) => {
                let (k, v) = kv.split_once('=')?;
                assert!(w.sysctl.contains_key(k), "unknown sysctl {k}");
                w.sysctl.insert(k.to_owned(), v.to_owned());
            }
            ("nft", ["-f", "-"]) => w.nft = Some(stdin),
            ("nft", ["delete", "table", "inet", "kith"]) => {
                if w.nft.take().is_none() {
                    return Some(fail(cmd, "Error: No such file or directory"));
                }
            }
            ("nft", ["-a", "list", "chain", "ip", "filter", "DOCKER-USER"]) => {
                let Some(rules) = &w.docker else {
                    return Some(fail(cmd, "Error: No such file or directory"));
                };
                let mut listing = String::new();
                for (handle, rule) in rules {
                    writeln!(listing, "\t\t{rule} # handle {handle}").ok()?;
                }
                return Some(ok(&listing, ""));
            }
            ("nft", ["insert", "rule", "ip", "filter", "DOCKER-USER", rule @ ..]) => {
                let rules = w.docker.as_mut()?;
                let handle = rules.keys().max().map_or(1, |h| h + 1);
                let rule = rule.join(" ").replace("comment kith", "comment \"kith\"");
                rules.insert(handle, rule);
            }
            (
                "nft",
                [
                    "delete",
                    "rule",
                    "ip",
                    "filter",
                    "DOCKER-USER",
                    "handle",
                    handle,
                ],
            ) => {
                let handle: u64 = handle.parse().ok()?;
                w.docker.as_mut()?.remove(&handle)?;
            }
            ("ip", [fam, "route", "show", "table", "all"]) => {
                return Some(ok(
                    w.route_table
                        .get(&(*fam == "-6"))
                        .map_or("", String::as_str),
                    "",
                ));
            }
            ("resolvectl", ["revert", dev]) => {
                w.resolved.retain(|k, _| !k.starts_with(&format!("{dev} ")));
            }
            ("resolvectl", [kind, dev, values @ ..]) => {
                w.resolved.insert(format!("{dev} {kind}"), values.join(" "));
            }
            ("resolvconf", ["-a", dev]) => drop(w.resolvconf.insert((*dev).to_owned(), stdin)),
            ("resolvconf", ["-d", dev]) => drop(w.resolvconf.remove(*dev)),
            _ => return None,
        }
        Some(ok("", ""))
    }

    fn macos(&mut self, cmd: &Cmd, args: &[&str], stdin: String) -> Option<Result<Output, Error>> {
        if matches!(cmd.program(), "networksetup" | "scutil") {
            return self.macos_dns(cmd, args);
        }
        let w = &mut self.world;
        match (cmd.program(), args) {
            ("route", ["-n", "get", rest @ ..]) => {
                if rest
                    .last()
                    .is_some_and(|dest| dest.starts_with("192.168.64."))
                {
                    return Some(ok(
                        "   route to: 192.168.64.2\n  interface: bridge100\n",
                        "",
                    ));
                }
                return Some(if rest.contains(&"-inet6") {
                    ok("gateway: fe80::1%en0\n  interface: en0\n", "")
                } else {
                    ok(
                        "   route to: default\n    gateway: 192.168.1.1\n  interface: en0\n",
                        "",
                    )
                });
            }
            ("route", ["-n", verb, rest @ ..]) => {
                let dest = rest
                    .iter()
                    .find(|a| a.parse::<IpPrefix>().is_ok() || a.parse::<IpAddr>().is_ok())?;
                let key = format!("mac {dest}");
                if *verb == "add" {
                    if w.routes.contains_key(&key) {
                        return Some(fail(cmd, "route: writing to routing socket: File exists"));
                    }
                    w.routes.insert(key, rest.join(" "));
                } else if w.routes.remove(&key).is_none() {
                    return Some(fail(cmd, "route: writing to routing socket: not in table"));
                }
            }
            ("pfctl", ["-a", "com.apple/kith", "-f", "-"]) => w.pf_anchor = Some(stdin),
            ("pfctl", ["-a", "com.apple/kith", "-F", "all"]) => w.pf_anchor = None,
            ("pfctl", ["-E"]) => {
                let token = (1..=u64::MAX).find(|t| !w.pf_tokens.contains(t))?;
                w.pf_tokens.insert(token);
                return Some(ok("", &format!("pf enabled\nToken : {token}\n")));
            }
            ("pfctl", ["-X", token]) => {
                if !w.pf_tokens.remove(&token.parse().unwrap_or(0)) {
                    return Some(fail(cmd, "pfctl: invalid token"));
                }
            }
            ("netstat", ["-rn", "-f", family]) => {
                return Some(ok(
                    w.route_table
                        .get(&(*family == "inet6"))
                        .map_or("", String::as_str),
                    "",
                ));
            }
            ("rmdir", [_]) => {}
            _ => return None,
        }
        Some(ok("", ""))
    }

    fn macos_dns(&mut self, cmd: &Cmd, args: &[&str]) -> Option<Result<Output, Error>> {
        let w = &mut self.world;
        match (cmd.program(), args) {
            ("networksetup", ["-listallnetworkservices"]) => {
                let mut names = String::new();
                for (name, _) in &w.services {
                    writeln!(names, "{name}").ok()?;
                }
                return Some(ok(
                    &format!(
                        "An asterisk (*) denotes that a network service is disabled.\n{names}"
                    ),
                    "",
                ));
            }
            ("networksetup", ["-getdnsservers", name]) => {
                let (_, servers) = w.services.iter().find(|(n, _)| n == name)?;
                let text = if servers.is_empty() {
                    format!("There aren't any DNS Servers set on {name}.\n")
                } else {
                    format!("{}\n", servers.replace(' ', "\n"))
                };
                return Some(ok(&text, ""));
            }
            ("networksetup", ["-setdnsservers", name, servers @ ..]) => {
                let slot = w.services.iter_mut().find(|(n, _)| n == name)?;
                slot.1 = if servers == ["Empty"] {
                    String::new()
                } else {
                    servers.join(" ")
                };
            }
            ("scutil", ["--dns"]) => {
                let primary = w
                    .services
                    .iter()
                    .find(|(name, servers)| !name.starts_with('*') && !servers.is_empty())
                    .map_or("192.168.1.1", |(_, servers)| servers.as_str());
                let mut lines = String::new();
                for (i, ip) in primary.split(' ').enumerate() {
                    writeln!(lines, "  nameserver[{i}] : {ip}").ok()?;
                }
                return Some(ok(
                    &format!(
                        "\nDNS configuration\n\nresolver #1\n{lines}  if_index : 15 (en0)\n\n\
                         resolver #2\n  domain   : local\n  options  : mdns\n"
                    ),
                    "",
                ));
            }
            _ => return None,
        }
        Some(ok("", ""))
    }
}

impl System for Fake {
    fn run(&mut self, cmd: &Cmd) -> Result<Output, Error> {
        self.log.push(cmd.to_string());
        match self.fail_in {
            Some(0) => {
                self.fail_in = None;
                return fail(cmd, "injected failure");
            }
            Some(n) => self.fail_in = Some(n - 1),
            None => {}
        }
        let args: Vec<&str> = cmd.argv().iter().map(String::as_str).collect();
        let stdin = cmd.input().unwrap_or_default().to_owned();
        let handled = match cmd.program() {
            "route" | "pfctl" | "networksetup" | "scutil" | "netstat" | "rmdir" => {
                self.macos(cmd, &args, stdin)
            }
            _ => self.linux(cmd, &args, stdin),
        };
        handled.unwrap_or_else(|| {
            self.unknown.push(cmd.to_string());
            ok("", "")
        })
    }

    fn read(&mut self, path: &Path) -> Result<Option<String>, Error> {
        Ok(self.world.files.get(path).cloned())
    }

    fn write(&mut self, path: &Path, contents: &str) -> Result<(), Error> {
        self.world
            .files
            .insert(path.to_owned(), contents.to_owned());
        Ok(())
    }

    fn remove(&mut self, path: &Path) -> Result<(), Error> {
        self.world.files.remove(path);
        Ok(())
    }

    fn list(&mut self, dir: &Path) -> Result<Vec<PathBuf>, Error> {
        Ok(self
            .world
            .files
            .keys()
            .filter(|p| p.parent() == Some(dir))
            .cloned()
            .collect())
    }

    fn has_program(&mut self, name: &str) -> bool {
        self.world.programs.contains(name)
    }
}

fn base_world() -> World {
    let mut w = World::default();
    for key in [
        "net.ipv4.ip_forward",
        "net.ipv6.conf.all.forwarding",
        "net.inet.ip.forwarding",
        "net.inet6.ip6.forwarding",
    ] {
        w.sysctl.insert(key.to_owned(), "0".to_owned());
    }
    w
}

fn full_plan(all_dns: bool) -> Plan {
    let mut plan = Plan::new(Interface::new("kith0").unwrap());
    plan.routes = vec![
        "100.64.0.0/10".parse().unwrap(),
        "fd7a::/48".parse().unwrap(),
    ];
    plan.exit_node = Some(ExitNode {
        peer_endpoints: vec!["203.0.113.7".parse().unwrap()],
    });
    plan.dns = Some(Dns {
        resolver: "100.100.100.100".parse().unwrap(),
        domains: vec![Domain::new("kith").unwrap()],
        all: all_dns,
    });
    plan.forwarding = true;
    plan.sources = vec!["100.70.0.2".parse().unwrap(), "fd7a::2".parse().unwrap()];
    plan.nat = Some(Nat {
        sources: vec!["100.64.0.0/10".parse().unwrap()],
    });
    plan
}

fn golden(platform: Platform, world: World, plan: &Plan, expected: &[&str]) {
    let mut sys = Sys::new(
        Fake {
            world,
            ..Fake::default()
        },
        platform,
        "/var/lib/kith/sys",
    );
    sys.apply(plan).unwrap();
    sys.revert(plan).unwrap();
    assert_eq!(sys.system().log, expected);
    assert_eq!(sys.system().unknown, Vec::<String>::new());
}

#[test]
fn linux_golden() {
    let mut world = base_world();
    world.programs.insert("resolvectl".to_owned());
    world.files.insert(
        "/run/systemd/resolve/io.systemd.Resolve".into(),
        String::new(),
    );
    world.docker = Some(BTreeMap::new());
    golden(
        Platform::Linux,
        world,
        &full_plan(true),
        &[
            "ip -4 route replace 100.64.0.0/10 dev kith0",
            "ip -6 route replace fd7a::/48 dev kith0",
            "ip -4 route replace default dev kith0 table 27497",
            "ip -4 rule del table main suppress_prefixlength 0 priority 5260",
            "ip -4 rule add table main suppress_prefixlength 0 priority 5260",
            "ip -4 rule del not fwmark 27497 table 27497 priority 5270",
            "ip -4 rule add not fwmark 27497 table 27497 priority 5270",
            "ip -6 route replace default dev kith0 table 27497",
            "ip -6 rule del table main suppress_prefixlength 0 priority 5260",
            "ip -6 rule add table main suppress_prefixlength 0 priority 5260",
            "ip -6 rule del not fwmark 27497 table 27497 priority 5270",
            "ip -6 rule add not fwmark 27497 table 27497 priority 5270",
            "resolvectl dns kith0 100.100.100.100",
            "resolvectl domain kith0 ~. ~kith",
            "resolvectl default-route kith0 true",
            "sysctl -n net.ipv4.ip_forward",
            "sysctl -n net.ipv6.conf.all.forwarding",
            "sysctl -w net.ipv4.ip_forward=1",
            "sysctl -w net.ipv6.conf.all.forwarding=1",
            "ip -4 route show default",
            "nft -f -",
            "nft -a list chain ip filter DOCKER-USER",
            "nft insert rule ip filter DOCKER-USER iifname kith0 accept comment kith",
            "nft insert rule ip filter DOCKER-USER oifname kith0 ct state established accept comment kith",
            "nft -a list chain ip filter DOCKER-USER",
            "nft delete rule ip filter DOCKER-USER handle 1",
            "nft delete rule ip filter DOCKER-USER handle 2",
            "nft delete table inet kith",
            "sysctl -w net.ipv4.ip_forward=0",
            "sysctl -w net.ipv6.conf.all.forwarding=0",
            "resolvectl revert kith0",
            "ip -4 rule del table main suppress_prefixlength 0 priority 5260",
            "ip -4 rule del not fwmark 27497 table 27497 priority 5270",
            "ip -4 route flush table 27497",
            "ip -6 rule del table main suppress_prefixlength 0 priority 5260",
            "ip -6 rule del not fwmark 27497 table 27497 priority 5270",
            "ip -6 route flush table 27497",
            "ip -4 route del 100.64.0.0/10 dev kith0",
            "ip -6 route del fd7a::/48 dev kith0",
        ],
    );
}

#[test]
fn macos_golden() {
    let mut world = base_world();
    world.services = vec![
        ("Wi-Fi".to_owned(), String::new()),
        ("USB 10/100 LAN".to_owned(), "192.0.2.53".to_owned()),
        ("*Bluetooth PAN".to_owned(), String::new()),
    ];
    golden(
        Platform::MacOs,
        world,
        &full_plan(true),
        &[
            "route -n delete -net 100.64.0.0/10 -interface kith0",
            "route -n add -net 100.64.0.0/10 -interface kith0 -ifa 100.70.0.2",
            "route -n delete -inet6 fd7a::/48 -interface kith0",
            "route -n add -inet6 fd7a::/48 -interface kith0 -ifa fd7a::2",
            "route -n delete -net 0.0.0.0/1 -interface kith0",
            "route -n delete -net 128.0.0.0/1 -interface kith0",
            "route -n delete -inet6 ::/1 -interface kith0",
            "route -n delete -inet6 8000::/1 -interface kith0",
            "route -n delete -host 203.0.113.7",
            "route -n get 203.0.113.7",
            "route -n add -host 203.0.113.7 192.168.1.1",
            "route -n add -net 0.0.0.0/1 -interface kith0 -ifa 100.70.0.2",
            "route -n add -net 128.0.0.0/1 -interface kith0 -ifa 100.70.0.2",
            "route -n add -inet6 ::/1 -interface kith0 -ifa fd7a::2",
            "route -n add -inet6 8000::/1 -interface kith0 -ifa fd7a::2",
            "scutil --dns",
            "networksetup -listallnetworkservices",
            "networksetup -getdnsservers Wi-Fi",
            "networksetup -getdnsservers USB 10/100 LAN",
            "networksetup -setdnsservers Wi-Fi 100.100.100.100",
            "networksetup -setdnsservers USB 10/100 LAN 100.100.100.100",
            "sysctl -n net.inet.ip.forwarding",
            "sysctl -n net.inet6.ip6.forwarding",
            "sysctl -w net.inet.ip.forwarding=1",
            "sysctl -w net.inet6.ip6.forwarding=1",
            "route -n get default",
            "pfctl -a com.apple/kith -f -",
            "pfctl -E",
            "pfctl -a com.apple/kith -F all",
            "pfctl -X 1",
            "sysctl -w net.inet.ip.forwarding=0",
            "sysctl -w net.inet6.ip6.forwarding=0",
            "networksetup -setdnsservers Wi-Fi Empty",
            "networksetup -setdnsservers USB 10/100 LAN 192.0.2.53",
            "rmdir /etc/resolver",
            "route -n delete -net 0.0.0.0/1 -interface kith0",
            "route -n delete -net 128.0.0.0/1 -interface kith0",
            "route -n delete -inet6 ::/1 -interface kith0",
            "route -n delete -inet6 8000::/1 -interface kith0",
            "route -n delete -host 203.0.113.7",
            "route -n delete -net 100.64.0.0/10 -interface kith0",
            "route -n delete -inet6 fd7a::/48 -interface kith0",
        ],
    );
}

#[test]
fn upstream_resolvers_skip_kith_and_survive_its_own_overrides() {
    let resolvers = |platform, world: World| {
        let mut sys = Sys::new(
            Fake {
                world,
                ..Fake::default()
            },
            platform,
            "/var/lib/kith/sys",
        );
        let found = sys.system_resolvers().unwrap();
        (sys, found)
    };
    let ips =
        |list: &[&str]| -> Vec<IpAddr> { list.iter().map(|ip| ip.parse().unwrap()).collect() };

    let mut world = base_world();
    world
        .files
        .insert("/etc/resolv.conf".into(), "nameserver 127.0.0.53\n".into());
    world.programs.insert("resolvectl".to_owned());
    world.files.insert(
        "/run/systemd/resolve/io.systemd.Resolve".into(),
        String::new(),
    );
    world.files.insert(
        "/run/systemd/resolve/resolv.conf".into(),
        "nameserver 10.0.0.2\nnameserver 100.100.100.100\nnameserver 2001:db8::53\n".into(),
    );
    assert_eq!(
        resolvers(Platform::Linux, world).1,
        ips(&["10.0.0.2", "2001:db8::53"])
    );

    let mut world = base_world();
    world
        .files
        .insert("/etc/resolv.conf".into(), "nameserver 192.0.2.1\n".into());
    world
        .programs
        .extend(["resolvectl".to_owned(), "resolvconf".to_owned()]);
    world.files.insert(
        "/run/systemd/resolve/stub-resolv.conf".into(),
        "nameserver 127.0.0.53\n".into(),
    );
    let (mut sys, before) = resolvers(Platform::Linux, world);
    sys.apply(&full_plan(false)).unwrap();
    assert!(
        sys.system().world.resolvconf.is_empty(),
        "systemd's resolvconf stand-in must not be used without resolved"
    );
    assert!(
        sys.system().world.resolved.is_empty(),
        "a leftover stub file does not mean resolved is running"
    );
    assert_eq!(
        sys.system_resolvers().unwrap(),
        before,
        "kith's rewrite must not hide the originals"
    );

    let mut world = base_world();
    world.services = vec![("Wi-Fi".to_owned(), String::new())];
    let (mut sys, before) = resolvers(Platform::MacOs, world);
    assert_eq!(
        before,
        ips(&["192.168.1.1"]),
        "DHCP-provided servers count too"
    );
    sys.apply(&full_plan(true)).unwrap();
    assert_eq!(
        sys.system_resolvers().unwrap(),
        before,
        "kith's override must not hide the originals"
    );
}

#[test]
fn routes_another_vpn_holds_in_kiths_ranges_are_reported() {
    let ranges: Vec<IpPrefix> = vec![
        "100.64.0.0/10".parse().unwrap(),
        "fd7a:115c:a1e0::/48".parse().unwrap(),
    ];
    let own = Interface::new("kith0").unwrap();
    let conflicts = |platform, v4: &str, v6: &str| {
        let mut world = base_world();
        world.route_table.insert(false, v4.to_owned());
        world.route_table.insert(true, v6.to_owned());
        let mut sys = Sys::new(
            Fake {
                world,
                ..Fake::default()
            },
            platform,
            "/var/lib/kith/sys",
        );
        let found = sys.route_conflicts(&ranges, &own).unwrap();
        found.iter().map(ToString::to_string).collect::<Vec<_>>()
    };

    let linux = conflicts(
        Platform::Linux,
        "default via 192.168.1.1 dev eth0\n100.64.0.0/10 dev kith0 scope link\n\
         100.100.100.100 dev tailscale0 table 52\nlocal 100.70.1.2 dev kith0 table local\n\
         10.0.0.0/8 dev eth1\n",
        "fd7a:115c:a1e0::/48 dev tailscale0 table 52 metric 1024\n",
    );
    assert_eq!(
        linux,
        [
            "100.100.100.100/32 via tailscale0",
            "fd7a:115c:a1e0::/48 via tailscale0"
        ]
    );

    let macos = conflicts(
        Platform::MacOs,
        "Routing tables\n\nInternet:\nDestination        Gateway            Flags  Netif Expire\n\
         default            192.168.29.1       UGScg  en0\n100.64/10          link#61            UCS    kith0\n\
         100.67.254.51      100.67.254.51      UH     utun37\n100.127.255.255    100.121.0.1        UH     kith0\n\
         127                127.0.0.1          UCS    lo0\n",
        "Routing tables\n\nInternet6:\nDestination   Gateway   Flags   Netif Expire\nfe80::%utun0/64  fe80::1%utun0  UcI  utun0\n",
    );
    assert_eq!(macos, ["100.67.254.51/32 via utun37"]);
}

/// An exit node on a directly connected network, such as a VM bridge, is pinned to that
/// interface; pinning it to the default gateway cut the tunnel to it.
#[test]
fn an_exit_on_a_local_bridge_keeps_its_own_route() {
    let mut plan = Plan::new(Interface::new("utun9").unwrap());
    plan.exit_node = Some(ExitNode {
        peer_endpoints: vec![
            "192.168.64.2".parse().unwrap(),
            "203.0.113.7".parse().unwrap(),
        ],
    });
    let mut sys = Sys::new(
        Fake {
            world: base_world(),
            ..Fake::default()
        },
        Platform::MacOs,
        "/var/lib/kith/sys",
    );
    sys.apply(&plan).unwrap();
    let routes = &sys.system().world.routes;
    assert_eq!(
        routes["mac 192.168.64.2"],
        "-host 192.168.64.2 -interface bridge100"
    );
    assert_eq!(routes["mac 203.0.113.7"], "-host 203.0.113.7 192.168.1.1");
}

#[test]
fn rejects_unsafe_arguments() {
    for bad in [
        "",
        "eth0;rm",
        "a b",
        "$(x)",
        "`x`",
        "a|b",
        "a&b",
        "x\n",
        "0123456789abcdef",
        "-x",
        ".hidden",
        "a/b",
    ] {
        assert!(Interface::new(bad).is_err(), "{bad:?}");
    }
    for bad in ["", "a/b", "..", "a..b", "-a.com", "a;b", "../etc", "a b"] {
        assert!(Domain::new(bad).is_err(), "{bad:?}");
    }
    for bad in [
        "10.0.0.1/8",
        "1.2.3.4/33",
        "1.2.3.4",
        "1.2.3.4/8;x",
        "::/129",
    ] {
        assert!(bad.parse::<IpPrefix>().is_err(), "{bad:?}");
    }
    for bad in ["a;b", "$x", "a b", "'", "\"", ">f", "*", ""] {
        assert!(!valid_arg(bad), "{bad:?}");
    }
    assert!(Interface::new("wg-0.1_a").is_ok() && Domain::new("Corp.Example-1").is_ok());
}

fn prefix() -> impl Strategy<Value = IpPrefix> {
    prop_oneof![
        (any::<[u8; 4]>(), 0..=32_u8).prop_map(|(a, l)| kith_types::Ipv4Prefix::truncating(
            a.into(),
            l
        )
        .unwrap()
        .into()),
        (any::<u128>(), 0..=128_u8).prop_map(|(a, l)| kith_types::Ipv6Prefix::truncating(
            a.into(),
            l
        )
        .unwrap()
        .into()),
    ]
}

fn plan() -> impl Strategy<Value = Plan> {
    let ip = any::<[u8; 4]>().prop_map(IpAddr::from);
    let ep = prop_oneof![
        ip.clone(),
        any::<u128>().prop_map(|a| IpAddr::from(core::net::Ipv6Addr::from(a)))
    ];
    let domain = "[a-z]{1,6}(\\.[a-z]{1,6}){0,2}".prop_map(|d| Domain::new(&d).unwrap());
    let dns = (ip, proptest::collection::vec(domain, 0..3), any::<bool>()).prop_map(
        |(resolver, domains, all)| Dns {
            resolver,
            domains,
            all,
        },
    );
    (
        "[a-z][a-z0-9]{0,14}",
        proptest::collection::vec(prefix(), 0..4),
        proptest::option::of(proptest::collection::vec(ep, 0..3)),
        proptest::option::of(dns),
        any::<bool>(),
        proptest::option::of(proptest::collection::vec(prefix(), 0..3)),
        proptest::collection::vec(any::<[u8; 4]>().prop_map(IpAddr::from), 0..3),
    )
        .prop_map(|(name, routes, exit, dns, forwarding, nat, sources)| Plan {
            interface: Interface::new(&name).unwrap(),
            routes,
            exit_node: exit.map(|peer_endpoints| ExitNode { peer_endpoints }),
            dns,
            forwarding,
            nat: nat.map(|sources| Nat { sources }),
            sources,
        })
}

fn world() -> impl Strategy<Value = World> {
    (
        any::<[bool; 4]>(),
        0..3_u8,
        proptest::option::of("[a-z ]{0,20}"),
        proptest::option::of("[a-z]{1,6}"),
        any::<bool>(),
        proptest::collection::vec(
            (
                "\\*?[A-Za-z][A-Za-z0-9 /-]{0,12}",
                proptest::option::of(any::<[u8; 4]>()),
            ),
            0..3,
        ),
    )
        .prop_map(
            |(sysctls, dns_backend, resolv_conf, foreign, docker, services)| {
                let mut w = base_world();
                w.docker = docker.then(BTreeMap::new);
                w.services = services
                    .into_iter()
                    .map(|(name, dns)| {
                        (
                            name,
                            dns.map(|ip| IpAddr::from(ip).to_string())
                                .unwrap_or_default(),
                        )
                    })
                    .collect();
                for (value, slot) in sysctls.iter().zip(w.sysctl.values_mut()) {
                    *slot = u8::from(*value).to_string();
                }
                match dns_backend {
                    0 => {
                        w.programs.insert("resolvectl".to_owned());
                        w.files.insert(
                            "/run/systemd/resolve/io.systemd.Resolve".into(),
                            String::new(),
                        );
                    }
                    1 => drop(w.programs.insert("resolvconf".to_owned())),
                    _ => {}
                }
                if let Some(text) = resolv_conf {
                    w.files.insert(
                        "/etc/resolv.conf".into(),
                        format!("nameserver 1.1.1.1\n{text}\n"),
                    );
                }
                if let Some(domain) = foreign {
                    w.files.insert(
                        Path::new("/etc/resolver").join(domain),
                        "nameserver 9.9.9.9\n".to_owned(),
                    );
                }
                w
            },
        )
}

proptest! {
    /// Whatever the plan and starting state, and even if `apply` fails part-way
    /// or runs twice (a restart), `revert` restores the original state.
    #[test]
    fn revert_restores_original_state(
        plan in plan(),
        world in world(),
        linux in any::<bool>(),
        twice in any::<bool>(),
    ) {
        let platform = if linux { Platform::Linux } else { Platform::MacOs };
        let mut sys = Sys::new(Fake { world: world.clone(), ..Fake::default() }, platform, "/var/lib/kith/sys");
        if sys.apply(&plan).is_ok() && twice {
            sys.apply(&plan).unwrap();
        }
        sys.revert(&plan).unwrap();
        sys.revert(&plan).unwrap();
        prop_assert!(sys.system().unknown.is_empty(), "{:?}", sys.system().unknown);
        prop_assert_eq!(&sys.system().world, &world);
    }

    /// Changing between any two plans, with any one command failing, either lands on the
    /// new plan or leaves exactly the old one; reverting afterwards restores the host.
    #[test]
    fn change_lands_on_the_new_plan_or_keeps_the_old(
        old in plan(),
        new in plan(),
        world in world(),
        linux in any::<bool>(),
        fail_in in proptest::option::of(0..48_usize),
    ) {
        let platform = if linux { Platform::Linux } else { Platform::MacOs };
        let mut sys = Sys::new(Fake { world: world.clone(), ..Fake::default() }, platform, "/var/lib/kith/sys");
        prop_assume!(sys.apply(&old).is_ok());
        let with_old = sys.system().world.clone();
        sys.system_mut().fail_in = fail_in;
        if sys.change(Some(&old), &new).is_err() {
            prop_assert_eq!(&sys.system().world, &with_old, "a failed change must keep the old plan");
        }
        sys.system_mut().fail_in = None;
        sys.revert(&new).unwrap();
        sys.revert(&old).unwrap();
        prop_assert!(sys.system().unknown.is_empty(), "{:?}", sys.system().unknown);
        prop_assert_eq!(&sys.system().world, &world);
    }
}

/// Writing a file that is a symlink, as `/etc/resolv.conf` often is, updates what it points
/// to and keeps the link; replacing the link broke DNS once kith restored the file.
#[cfg(unix)]
#[test]
fn writes_go_through_symlinks_and_keep_them() {
    let dir = std::env::temp_dir().join(format!("kith-sys-link-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("run")).unwrap();
    let target = dir.join("run/stub-resolv.conf");
    let link = dir.join("resolv.conf");
    std::fs::write(&target, "nameserver 127.0.0.53\n").unwrap();
    std::os::unix::fs::symlink("run/stub-resolv.conf", &link).unwrap();

    crate::OsSystem
        .write(&link, "nameserver 100.64.0.1\n")
        .unwrap();
    let still_link = std::fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink();
    let written = std::fs::read_to_string(&target).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(still_link, "the link must stay a link");
    assert_eq!(written, "nameserver 100.64.0.1\n");
}

#[test]
fn an_exit_masquerades_the_mesh_out_of_the_default_route() {
    for (platform, rules) in [
        (
            Platform::Linux,
            [
                "oifname \"eth0\" ip saddr 100.64.0.0/10 masquerade",
                "oifname \"eth0\" ip6 saddr fd7a::/48 masquerade",
            ],
        ),
        (
            Platform::MacOs,
            [
                "nat on en0 inet from 100.64.0.0/10 to any -> (en0)",
                "nat on en0 inet6 from fd7a::/48 to any -> (en0)",
            ],
        ),
    ] {
        let mut sys = Sys::new(
            Fake {
                world: base_world(),
                ..Fake::default()
            },
            platform,
            "/var/lib/kith/sys",
        );
        let mut plan = full_plan(false);
        plan.nat = Some(Nat {
            sources: vec![
                "100.64.0.0/10".parse().unwrap(),
                "fd7a::/48".parse().unwrap(),
            ],
        });
        sys.apply(&plan).unwrap();
        let w = &sys.system().world;
        let loaded = w.nft.clone().or_else(|| w.pf_anchor.clone()).unwrap();
        for rule in rules {
            assert!(
                loaded.contains(rule),
                "{platform:?} is missing `{rule}` in:\n{loaded}"
            );
        }
        sys.revert(&plan).unwrap();
        let w = &sys.system().world;
        assert!(
            w.nft.is_none() && w.pf_anchor.as_deref().is_none_or(str::is_empty),
            "{platform:?} left NAT behind"
        );
    }
}
