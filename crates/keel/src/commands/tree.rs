//! `keel tree` — print the resolved dependency graph of each workspace
//! member (offline: reads `Keel.lock` when present).
//!
//! Contract: `docs/rewrite/UPDATE_TREE.md`.

use std::collections::BTreeSet;
use std::path::Path;

/// A node's version + source tag, resolved from the lockfile / manifests.
struct Node {
    version: String,
    tag: &'static str,
}

/// Look up a dependency's label: member > lock > declared.
fn node_for(
    name: &str,
    req: &str,
    lock: Option<&lpp_pm::Lock>,
    member_versions: &BTreeSet<String>,
    ws: &lpp_pm::Workspace,
) -> Node {
    // A dep named after a workspace member IS the member.
    if member_versions.contains(name) {
        return Node {
            version: member_version_for(name, ws).unwrap_or_else(|| "?".to_string()),
            tag: "member",
        };
    }
    if let Some(p) = lock
        .map(|l| l.packages.iter().find(|p| p.name == name))
        .flatten()
    {
        return Node {
            version: p.version.clone(),
            tag: if p.source == "registry" {
                "registry"
            } else {
                "path"
            },
        };
    }
    // Declared but not locked: path deps still label cleanly.
    let is_path = ws.members.iter().any(|m| {
        m.manifest
            .dependencies
            .iter()
            .any(|(n, d)| n == name && d.path().is_some())
    });
    Node {
        version: if is_path {
            member_version_for(name, ws).unwrap_or_else(|| "?".to_string())
        } else {
            format!("? ({req})")
        },
        tag: if is_path { "path" } else { "unlocked" },
    }
}

/// The manifest version of a (possibly transitive) member named `name`.
fn member_version_for(name: &str, ws: &lpp_pm::Workspace) -> Option<String> {
    ws.members
        .iter()
        .find(|m| m.name() == name)
        .map(|m| m.manifest.version().to_string())
}

/// Recursively render one dependency subtree (deterministic; cycle-safe).
fn render_node(
    out: &mut String,
    name: &str,
    req: &str,
    prefix: &str,
    is_last: bool,
    branch: &mut BTreeSet<String>,
    lock: Option<&lpp_pm::Lock>,
    member_versions: &BTreeSet<String>,
    ws: &lpp_pm::Workspace,
    unlocked_seen: &mut bool,
) {
    let node = node_for(name, req, lock, member_versions, ws);
    if node.tag == "unlocked" {
        *unlocked_seen = true;
    }
    // Members are shared roots, never a cycle in their own right.
    let cyclic = node.tag != "member" && !branch.insert(name.to_string());
    let connector = if is_last { "└── " } else { "├── " };
    let version = if cyclic {
        format!("{} (cyclic)", node.version)
    } else {
        node.version.clone()
    };
    out.push_str(&format!(
        "{prefix}{connector}{name} v{version} ({})\n",
        node.tag
    ));

    // Children: the lock's dependency names (declaration order).
    let children: Vec<String> = lock
        .map(|l| {
            l.packages
                .iter()
                .find(|p| p.name == name)
                .map(|p| p.deps.clone())
                .unwrap_or_default()
        })
        .unwrap_or_default();
    for (i, child) in children.iter().enumerate() {
        let child_prefix = format!("{prefix}{}", if is_last { "    " } else { "│   " });
        render_node(
            out,
            child,
            "*",
            &child_prefix,
            i + 1 == children.len(),
            branch,
            lock,
            member_versions,
            ws,
            unlocked_seen,
        );
    }
    branch.remove(name);
}

/// Build the full `keel tree` output. `only` restricts to one member name.
pub fn render(dir: &Path, only: Option<&str>) -> Result<String, String> {
    let mut text = String::new();
    render_into(&mut text, dir, only)?;
    Ok(text)
}

/// The `keel tree` implementation (render + print).
pub fn tree(dir: &Path, only: Option<&str>) -> Result<(), String> {
    let text = render(dir, only)?;
    print!("{text}");
    Ok(())
}

fn render_into(text: &mut String, dir: &Path, only: Option<&str>) -> Result<(), String> {
    let ws = lpp_pm::Workspace::discover(dir).map_err(|e| e.to_string())?;

    let lock: Option<lpp_pm::Lock> = ws
        .root
        .join("Keel.lock")
        .is_file()
        .then(|| {
            std::fs::read_to_string(ws.root.join("Keel.lock"))
                .ok()
                .and_then(|doc| lpp_pm::Lock::parse(&doc).ok())
        })
        .flatten();

    let member_names: BTreeSet<String> = ws.members.iter().map(|m| m.name().to_string()).collect();

    let members: Vec<&lpp_pm::Member> = match only {
        Some(name) => ws.members.iter().filter(|m| m.name() == name).collect(),
        None => ws.members.iter().collect(),
    };
    if members.is_empty() {
        return Err(match only {
            Some(name) => format!("no workspace member named '{name}'"),
            None => "empty workspace".to_string(),
        });
    }

    let mut unlocked_any = false;
    for (mi, m) in members.iter().enumerate() {
        let mut out = String::new();
        let mut unlocked_here = false;
        let deps: Vec<(String, String)> = m
            .manifest
            .dependencies
            .iter()
            .map(|(n, d)| (n.clone(), d.version().to_string()))
            .collect();
        for (i, (name, req)) in deps.iter().enumerate() {
            let mut branch = BTreeSet::new();
            branch.insert(m.name().to_string());
            render_node(
                &mut out,
                name,
                req,
                "",
                i + 1 == deps.len(),
                &mut branch,
                lock.as_ref(),
                &member_names,
                &ws,
                &mut unlocked_here,
            );
        }
        if deps.is_empty() {
            out.push_str("(no dependencies)\n");
        }
        unlocked_any |= unlocked_here;
        if mi > 0 {
            text.push('\n');
        }
        text.push_str(&format!("{} v{}\n", m.name(), m.manifest.version()));
        text.push_str(&out);
    }

    if lock.is_none() && unlocked_any {
        text.push('\n');
        text.push_str(
            "note: some registry dependencies are unlocked — run `keel fetch` to build Keel.lock\n",
        );
    }
    Ok(())
}
