use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use kith_sys::{Cmd, Dns, Domain, Error, ExitNode, Interface, Output, Plan, Platform, System};
use kith_types::IpPrefix;

use super::{DynSystem, OsPlanner};

/// The Linux state kith's commands leave behind, modelled closely enough to spot leftovers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Host {
    routes: BTreeSet<String>,
    exit_table: BTreeSet<String>,
    rules: BTreeSet<String>,
    nat: bool,
    files: BTreeMap<PathBuf, String>,
}

#[derive(Clone, Default)]
struct FakeLinux {
    host: Arc<Mutex<Host>>,
    fail_on: Arc<Mutex<Option<&'static str>>>,
}

impl FakeLinux {
    fn host(&self) -> Host {
        lock(&self.host).clone()
    }

    fn fail_on(&self, needle: Option<&'static str>) {
        *lock(&self.fail_on) = needle;
    }
}

impl System for FakeLinux {
    fn run(&mut self, cmd: &Cmd) -> Result<Output, Error> {
        let line = cmd.to_string();
        if let Some(needle) = *lock(&self.fail_on)
            && line.contains(needle)
        {
            return Err(Error::Command {
                command: line,
                code: Some(2),
                stderr: "injected failure".into(),
            });
        }
        let mut host = lock(&self.host);
        let words: Vec<&str> = line.split(' ').collect();
        let mut stdout = String::new();
        match words.as_slice() {
            ["ip", family, "rule", "add", rule @ ..] => {
                host.rules.insert(format!("{family} {}", rule.join(" ")));
            }
            ["ip", family, "rule", "del", rule @ ..] => {
                host.rules.remove(&format!("{family} {}", rule.join(" ")));
            }
            [
                "ip",
                family,
                "route",
                "replace",
                "default",
                "dev",
                _,
                "table",
                _,
            ] => {
                host.exit_table.insert((*family).to_owned());
            }
            ["ip", family, "route", "flush", "table", _] => {
                host.exit_table.remove(*family);
            }
            ["ip", _, "route", "replace", prefix, "dev", _] => {
                host.routes.insert((*prefix).to_owned());
            }
            ["ip", _, "route", "del", prefix, "dev", _] => {
                host.routes.remove(*prefix);
            }
            ["ip", _, "route", "show", "default"] => {
                stdout = "default via 192.0.2.1 dev eth0 proto dhcp\n".into();
            }
            ["sysctl", "-n", _] => stdout = "0\n".into(),
            ["nft", "-f", "-"] => host.nat = true,
            ["nft", "delete", "table", "inet", "kith"] => host.nat = false,
            _ => {}
        }
        Ok(Output {
            stdout,
            stderr: String::new(),
        })
    }

    fn read(&mut self, path: &Path) -> Result<Option<String>, Error> {
        Ok(lock(&self.host).files.get(path).cloned())
    }

    fn write(&mut self, path: &Path, contents: &str) -> Result<(), Error> {
        let mut host = lock(&self.host);
        host.files.insert(path.to_owned(), contents.to_owned());
        Ok(())
    }

    fn remove(&mut self, path: &Path) -> Result<(), Error> {
        lock(&self.host).files.remove(path);
        Ok(())
    }

    fn list(&mut self, _dir: &Path) -> Result<Vec<PathBuf>, Error> {
        Ok(Vec::new())
    }

    fn has_program(&mut self, _name: &str) -> bool {
        false
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

const RESOLV_CONF: &str = "nameserver 192.0.2.53\n";

fn planner() -> (OsPlanner, FakeLinux) {
    let fake = FakeLinux::default();
    lock(&fake.host)
        .files
        .insert("/etc/resolv.conf".into(), RESOLV_CONF.into());
    let planner = OsPlanner::new(
        DynSystem(Box::new(fake.clone())),
        Platform::Linux,
        Interface::new("tun0").unwrap(),
        Path::new("/var/lib/kith"),
    );
    (planner, fake)
}

fn mesh_plan(exit: bool) -> Plan {
    let mut plan = Plan::new(Interface::new("tun0").unwrap());
    plan.routes = vec!["100.64.0.0/10".parse::<IpPrefix>().unwrap()];
    plan.exit_node = exit.then(ExitNode::default);
    plan.dns = Some(Dns {
        resolver: "100.64.0.1".parse().unwrap(),
        domains: vec![Domain::new("home.kith.internal").unwrap()],
        all: exit,
    });
    plan
}

#[test]
fn turning_the_exit_off_removes_its_rules_and_stopping_restores_the_host() {
    let (mut planner, fake) = planner();
    let pristine = fake.host();

    planner.apply(mesh_plan(false)).unwrap();
    let mesh_only = fake.host();
    assert!(mesh_only.rules.is_empty() && mesh_only.routes.contains("100.64.0.0/10"));

    planner.apply(mesh_plan(true)).unwrap();
    let with_exit = fake.host();
    assert_eq!(with_exit.rules.len(), 4, "two rules per address family");
    assert_eq!(with_exit.exit_table.len(), 2);

    planner.apply(mesh_plan(false)).unwrap();
    assert_eq!(
        fake.host(),
        mesh_only,
        "exit off must leave the mesh-only state"
    );

    planner.revert().unwrap();
    assert_eq!(fake.host(), pristine, "stopping must leave nothing behind");
}

#[test]
fn a_plan_that_fails_halfway_rolls_back_to_the_previous_one() {
    let (mut planner, fake) = planner();
    planner.apply(mesh_plan(false)).unwrap();
    let before = fake.host();

    fake.fail_on(Some("rule add"));
    assert!(planner.apply(mesh_plan(true)).is_err());
    fake.fail_on(None);
    assert_eq!(
        fake.host(),
        before,
        "a failed change must not leave half of itself"
    );

    planner.apply(mesh_plan(true)).unwrap();
    assert_eq!(
        fake.host().rules.len(),
        4,
        "the change applies once the failure clears"
    );
}
