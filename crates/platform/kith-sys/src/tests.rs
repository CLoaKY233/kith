use std::{
    collections::{BTreeMap, BTreeSet},
    net::IpAddr,
    path::{Path, PathBuf},
};

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
}

#[derive(Debug, Default)]
struct Fake {
    world: World,
    log: Vec<String>,
    unknown: Vec<String>,
    next_token: u64,
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

impl System for Fake {
    #[expect(
        clippy::panic_in_result_fn,
        reason = "the fake asserts on impossible commands"
    )]
    fn run(&mut self, cmd: &Cmd) -> Result<Output, Error> {
        self.log.push(cmd.to_string());
        let w = &mut self.world;
        let args: Vec<&str> = cmd.argv().iter().map(String::as_str).collect();
        let stdin = cmd.input().unwrap_or_default().to_owned();
        match (cmd.program(), args.as_slice()) {
            ("ip", [fam, "route", "replace", dst, "dev", dev, rest @ ..]) => {
                let table = rest.get(1).unwrap_or(&"main");
                w.routes
                    .insert(format!("{fam} {dst} {table}"), (*dev).to_owned());
            }
            ("ip", [fam, "route", "del", dst, "dev", _]) => {
                if w.routes.remove(&format!("{fam} {dst} main")).is_none() {
                    return fail(cmd, "RTNETLINK answers: No such process");
                }
            }
            ("ip", [fam, "route", "flush", "table", t]) => {
                w.routes
                    .retain(|k, _| !(k.starts_with(fam) && k.ends_with(&format!(" {t}"))));
            }
            ("ip", ["-4", "route", "show", "default"]) => {
                return ok("default via 192.168.1.1 dev eth0 proto dhcp\n", "");
            }
            ("ip", [fam, "rule", op, spec @ ..]) => {
                let key = format!("{fam} {}", spec.join(" "));
                match *op {
                    "add" => w.rules.push(key),
                    _ => match w.rules.iter().position(|r| *r == key) {
                        Some(i) => drop(w.rules.remove(i)),
                        None => return fail(cmd, "RTNETLINK answers: No such file or directory"),
                    },
                }
            }
            ("sysctl", ["-n", key]) => return ok(&format!("{}\n", w.sysctl[*key]), ""),
            ("sysctl", ["-w", kv]) => {
                let (k, v) = kv.split_once('=').unwrap();
                assert!(w.sysctl.contains_key(k), "unknown sysctl {k}");
                w.sysctl.insert(k.to_owned(), v.to_owned());
            }
            ("nft", ["-f", "-"]) => w.nft = Some(stdin),
            ("nft", ["delete", "table", "inet", "kith"]) => {
                if w.nft.take().is_none() {
                    return fail(cmd, "Error: No such file or directory");
                }
            }
            ("resolvectl", ["revert", dev]) => {
                w.resolved.retain(|k, _| !k.starts_with(&format!("{dev} ")));
            }
            ("resolvectl", [kind, dev, values @ ..]) => {
                w.resolved.insert(format!("{dev} {kind}"), values.join(" "));
            }
            ("resolvconf", ["-a", dev]) => drop(w.resolvconf.insert((*dev).to_owned(), stdin)),
            ("resolvconf", ["-d", dev]) => drop(w.resolvconf.remove(*dev)),
            ("route", ["-n", "get", rest @ ..]) => {
                return if rest.contains(&"-inet6") {
                    ok("gateway: fe80::1%en0\n  interface: en0\n", "")
                } else {
                    ok(
                        "   route to: default\n    gateway: 192.168.1.1\n  interface: en0\n",
                        "",
                    )
                };
            }
            ("route", ["-n", verb, rest @ ..]) => {
                let dest = rest
                    .iter()
                    .find(|a| a.parse::<IpPrefix>().is_ok() || a.parse::<IpAddr>().is_ok())
                    .unwrap();
                let key = format!("mac {dest}");
                if *verb == "add" {
                    if w.routes.contains_key(&key) {
                        return fail(cmd, "route: writing to routing socket: File exists");
                    }
                    w.routes.insert(key, rest.join(" "));
                } else if w.routes.remove(&key).is_none() {
                    return fail(cmd, "route: writing to routing socket: not in table");
                }
            }
            ("pfctl", ["-a", "com.apple/kith", "-f", "-"]) => w.pf_anchor = Some(stdin),
            ("pfctl", ["-a", "com.apple/kith", "-F", "all"]) => w.pf_anchor = None,
            ("pfctl", ["-E"]) => {
                self.next_token += 1;
                w.pf_tokens.insert(self.next_token);
                return ok("", &format!("pf enabled\nToken : {}\n", self.next_token));
            }
            ("pfctl", ["-X", token]) => {
                if !w.pf_tokens.remove(&token.parse().unwrap_or(0)) {
                    return fail(cmd, "pfctl: invalid token");
                }
            }
            ("rmdir", [_]) => {}
            _ => self.unknown.push(cmd.to_string()),
        }
        ok("", "")
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
    golden(
        Platform::MacOs,
        base_world(),
        &full_plan(false),
        &[
            "route -n delete -net 100.64.0.0/10 -interface kith0",
            "route -n add -net 100.64.0.0/10 -interface kith0",
            "route -n delete -inet6 fd7a::/48 -interface kith0",
            "route -n add -inet6 fd7a::/48 -interface kith0",
            "route -n delete -net 0.0.0.0/1 -interface kith0",
            "route -n delete -net 128.0.0.0/1 -interface kith0",
            "route -n delete -inet6 ::/1 -interface kith0",
            "route -n delete -inet6 8000::/1 -interface kith0",
            "route -n get default",
            "route -n delete -host 203.0.113.7",
            "route -n add -host 203.0.113.7 192.168.1.1",
            "route -n add -net 0.0.0.0/1 -interface kith0",
            "route -n add -net 128.0.0.0/1 -interface kith0",
            "route -n add -inet6 ::/1 -interface kith0",
            "route -n add -inet6 8000::/1 -interface kith0",
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
    )
        .prop_map(|(name, routes, exit, dns, forwarding, nat)| Plan {
            interface: Interface::new(&name).unwrap(),
            routes,
            exit_node: exit.map(|peer_endpoints| ExitNode { peer_endpoints }),
            dns,
            forwarding,
            nat: nat.map(|sources| Nat { sources }),
        })
}

fn world() -> impl Strategy<Value = World> {
    (
        any::<[bool; 4]>(),
        0..3_u8,
        proptest::option::of("[a-z ]{0,20}"),
        proptest::option::of("[a-z]{1,6}"),
    )
        .prop_map(|(sysctls, dns_backend, resolv_conf, foreign)| {
            let mut w = base_world();
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
        })
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
}
