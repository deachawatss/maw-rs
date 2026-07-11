const PANE_TREE_PID_FORMAT: &str = "#{pane_pid}";
const PANE_TREE_REAP_GRACE: std::time::Duration = std::time::Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessSignal {
    Terminate,
    Kill,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessEntry {
    pid: u32,
    parent_pid: u32,
    started_at: String,
}

trait PaneTreeRuntime {
    fn pane_tree_snapshot(&mut self) -> Result<Vec<ProcessEntry>, String>;
    fn pane_tree_signal(&mut self, signal: ProcessSignal, pids: &[u32]) -> Result<(), String>;
    fn pane_tree_wait_grace(&mut self) -> Result<(), String>;
}

struct SystemPaneTreeRuntime;

impl PaneTreeRuntime for SystemPaneTreeRuntime {
    fn pane_tree_snapshot(&mut self) -> Result<Vec<ProcessEntry>, String> {
        let output = std::process::Command::new("ps")
            .args(["-axo", "pid=,ppid=,lstart="])
            .output()
            .map_err(|error| format!("pane reaper: run ps failed: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "pane reaper: ps failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(pane_tree_parse_processes(&String::from_utf8_lossy(
            &output.stdout,
        )))
    }

    fn pane_tree_signal(&mut self, signal: ProcessSignal, pids: &[u32]) -> Result<(), String> {
        let signal_name = match signal {
            ProcessSignal::Terminate => "-TERM",
            ProcessSignal::Kill => "-KILL",
        };
        for pid in pids {
            let output = std::process::Command::new("/bin/kill")
                .args([signal_name, &pid.to_string()])
                .output()
                .map_err(|error| format!("pane reaper: signal {pid} failed to start: {error}"))?;
            if !output.status.success() {
                return Err(format!(
                    "pane reaper: signal {pid} failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
        }
        Ok(())
    }

    fn pane_tree_wait_grace(&mut self) -> Result<(), String> {
        std::thread::sleep(PANE_TREE_REAP_GRACE);
        Ok(())
    }
}

/// Reap the descendants of the supplied tmux pane roots without signalling the pane roots.
///
/// `tmux` performs the final shell cleanup. Keeping the roots out of these signal lists makes
/// the process boundary explicit: only processes proven to descend from the selected pane PIDs
/// are touched here.
fn reap_pane_descendants(
    runtime: &mut impl PaneTreeRuntime,
    pane_pids: &[u32],
) -> Result<(), String> {
    let initial_snapshot = runtime.pane_tree_snapshot()?;
    let descendants = pane_tree_descendants(&initial_snapshot, pane_pids);
    if descendants.is_empty() {
        return Ok(());
    }
    let initial_descendants = pane_tree_entries(&initial_snapshot, &descendants);
    runtime.pane_tree_signal(ProcessSignal::Terminate, &descendants)?;
    runtime.pane_tree_wait_grace()?;

    let after_grace = runtime.pane_tree_snapshot()?;
    let mut survivors = pane_tree_descendants(&after_grace, pane_pids)
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    survivors.extend(pane_tree_original_survivors(
        &initial_descendants,
        &after_grace,
    ));
    let survivors = survivors.into_iter().collect::<Vec<_>>();
    if !survivors.is_empty() {
        runtime.pane_tree_signal(ProcessSignal::Kill, &survivors)?;
    }
    Ok(())
}

/// Resolve the exact panes for one already-validated tmux target, then reap their descendants.
fn reap_tmux_target<R: maw_tmux::TmuxRunner>(runner: &mut R, target: &str) -> Result<(), String> {
    let raw = runner
        .run(
            "list-panes",
            &[
                "-t".to_owned(),
                target.to_owned(),
                "-F".to_owned(),
                PANE_TREE_PID_FORMAT.to_owned(),
            ],
        )
        .map_err(|error| {
            format!(
                "pane reaper: list panes for {target} failed: {}",
                error.message
            )
        })?;
    let pane_pids = pane_tree_parse_pids(&raw);
    if pane_pids.is_empty() {
        return Ok(());
    }
    reap_pane_descendants(&mut SystemPaneTreeRuntime, &pane_pids)
}

fn pane_tree_parse_pids(raw: &str) -> Vec<u32> {
    let mut pids = raw
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .filter(|pid| *pid > 1)
        .collect::<Vec<_>>();
    pids.sort_unstable();
    pids.dedup();
    pids
}

fn pane_tree_parse_processes(raw: &str) -> Vec<ProcessEntry> {
    raw.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse::<u32>().ok()?;
            let parent_pid = fields.next()?.parse::<u32>().ok()?;
            let started_at = fields.collect::<Vec<_>>().join(" ");
            (pid > 1 && !started_at.is_empty()).then_some(ProcessEntry {
                pid,
                parent_pid,
                started_at,
            })
        })
        .collect()
}

fn pane_tree_entries(processes: &[ProcessEntry], pids: &[u32]) -> Vec<ProcessEntry> {
    let pids = pids
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    processes
        .iter()
        .filter(|process| pids.contains(&process.pid))
        .cloned()
        .collect()
}

fn pane_tree_original_survivors(initial: &[ProcessEntry], current: &[ProcessEntry]) -> Vec<u32> {
    let current = current
        .iter()
        .map(|process| (process.pid, process.started_at.as_str()))
        .collect::<std::collections::BTreeMap<_, _>>();
    initial
        .iter()
        .filter(|process| {
            current
                .get(&process.pid)
                .is_some_and(|started_at| *started_at == process.started_at.as_str())
        })
        .map(|process| process.pid)
        .collect()
}

fn pane_tree_descendants(processes: &[ProcessEntry], roots: &[u32]) -> Vec<u32> {
    let roots = roots
        .iter()
        .copied()
        .filter(|pid| *pid > 1)
        .collect::<std::collections::BTreeSet<_>>();
    if roots.is_empty() {
        return Vec::new();
    }

    let mut children = std::collections::BTreeMap::<u32, Vec<u32>>::new();
    for process in processes {
        if process.pid > 1 && process.parent_pid > 0 {
            children
                .entry(process.parent_pid)
                .or_default()
                .push(process.pid);
        }
    }
    for values in children.values_mut() {
        values.sort_unstable();
        values.dedup();
    }

    let mut pending = std::collections::VecDeque::new();
    for root in &roots {
        pending.push_back((*root, 0_u32));
    }
    let mut found = std::collections::BTreeMap::<u32, u32>::new();
    while let Some((parent, depth)) = pending.pop_front() {
        for child in children.get(&parent).into_iter().flatten() {
            if roots.contains(child) || found.contains_key(child) {
                continue;
            }
            found.insert(*child, depth + 1);
            pending.push_back((*child, depth + 1));
        }
    }

    let mut descendants = found.into_iter().collect::<Vec<_>>();
    descendants.sort_unstable_by(|(left_pid, left_depth), (right_pid, right_depth)| {
        right_depth
            .cmp(left_depth)
            .then_with(|| left_pid.cmp(right_pid))
    });
    descendants.into_iter().map(|(pid, _)| pid).collect()
}

#[cfg(test)]
mod pane_tree_reaper_tests {
    use super::*;

    struct FakeRuntime {
        snapshots: std::collections::VecDeque<Vec<ProcessEntry>>,
        signals: Vec<(ProcessSignal, Vec<u32>)>,
    }

    impl FakeRuntime {
        fn with_snapshots(snapshots: Vec<Vec<ProcessEntry>>) -> Self {
            Self {
                snapshots: snapshots.into(),
                signals: Vec::new(),
            }
        }
    }

    impl PaneTreeRuntime for FakeRuntime {
        fn pane_tree_snapshot(&mut self) -> Result<Vec<ProcessEntry>, String> {
            self.snapshots
                .pop_front()
                .ok_or_else(|| "missing fake process snapshot".to_owned())
        }

        fn pane_tree_signal(&mut self, signal: ProcessSignal, pids: &[u32]) -> Result<(), String> {
            self.signals.push((signal, pids.to_vec()));
            Ok(())
        }

        fn pane_tree_wait_grace(&mut self) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn reaps_only_the_target_panes_full_descendant_tree() {
        let mut runtime = FakeRuntime::with_snapshots(vec![
            vec![
                ProcessEntry {
                    pid: 10,
                    parent_pid: 1,
                    started_at: "started-10".to_owned(),
                },
                ProcessEntry {
                    pid: 11,
                    parent_pid: 10,
                    started_at: "started-11".to_owned(),
                },
                ProcessEntry {
                    pid: 12,
                    parent_pid: 11,
                    started_at: "started-12".to_owned(),
                },
                ProcessEntry {
                    pid: 13,
                    parent_pid: 12,
                    started_at: "started-13".to_owned(),
                },
                ProcessEntry {
                    pid: 20,
                    parent_pid: 1,
                    started_at: "started-20".to_owned(),
                },
                ProcessEntry {
                    pid: 21,
                    parent_pid: 20,
                    started_at: "started-21".to_owned(),
                },
            ],
            vec![
                ProcessEntry {
                    pid: 10,
                    parent_pid: 1,
                    started_at: "started-10".to_owned(),
                },
                ProcessEntry {
                    pid: 13,
                    parent_pid: 1,
                    started_at: "started-13".to_owned(),
                },
                ProcessEntry {
                    pid: 20,
                    parent_pid: 1,
                    started_at: "started-20".to_owned(),
                },
                ProcessEntry {
                    pid: 21,
                    parent_pid: 20,
                    started_at: "started-21".to_owned(),
                },
            ],
        ]);

        reap_pane_descendants(&mut runtime, &[10]).expect("reap pane descendants");

        assert_eq!(
            runtime.signals,
            vec![
                (ProcessSignal::Terminate, vec![13, 12, 11]),
                (ProcessSignal::Kill, vec![13]),
            ]
        );
    }

    #[test]
    fn skips_a_pane_without_live_descendants() {
        let mut runtime = FakeRuntime::with_snapshots(vec![vec![ProcessEntry {
            pid: 10,
            parent_pid: 1,
            started_at: "started-10".to_owned(),
        }]]);

        reap_pane_descendants(&mut runtime, &[10]).expect("skip empty pane tree");

        assert!(runtime.signals.is_empty());
    }
}
