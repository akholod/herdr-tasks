//! One function per subcommand.

use super::output::{self, print_json, project_json, task_block, task_json, task_line};
use super::{Cli, Command, CreateArgs, ListArgs, SyncArgs, UpdateArgs};
use crate::config::{self, Config, SyncMode};
use crate::error::{AppError, Result};
use crate::store::task::{self, NewTask, TaskEntry, TaskPatch};
use crate::store::{Project, Store};
use crate::sync::{self, RunOptions, Selection, Session};
use serde_json::json;
use std::io::{Read, Write};
use std::path::PathBuf;

struct Ctx {
    store: Store,
    project: Project,
    cfg: Config,
    json: bool,
}

fn ctx(cli: &Cli) -> Result<Ctx> {
    let store = Store::resolve();
    let dir = match &cli.project {
        Some(p) => p.clone(),
        None => std::env::current_dir()
            .map_err(|e| AppError::Config(format!("current directory: {e}")))?,
    };
    let project = store.project_for_dir(&dir)?;
    let cfg = Config::load(&store.root, Some(&project.folder))?;
    Ok(Ctx {
        store,
        project,
        cfg,
        json: cli.json,
    })
}

pub fn run(cli: Cli) -> Result<()> {
    match &cli.command {
        Command::Skill => {
            let mut out = std::io::stdout().lock();
            out.write_all(crate::skill::SKILL_MD)
                .map_err(|e| AppError::Config(format!("stdout: {e}")))?;
            Ok(())
        }
        Command::Tui { surface } => crate::tui::run(*surface),
        Command::Create(a) => create(&ctx(&cli)?, a),
        Command::List(a) => list(&cli, a),
        Command::Show { id, details } => show(&ctx(&cli)?, id, *details),
        Command::Update(a) => update(&ctx(&cli)?, a),
        Command::Move { id, status } => {
            let c = ctx(&cli)?;
            let t = task::move_task(&c.project, &c.cfg, id, status)?;
            print_task(&c, &t, &format!("{} -> {}", t.meta.id, t.meta.status));
            Ok(())
        }
        Command::Pin { id, off } => {
            let c = ctx(&cli)?;
            let t = task::set_pin(&c.project, &c.cfg, id, !off)?;
            let what = if t.meta.pinned { "pinned" } else { "unpinned" };
            print_task(&c, &t, &format!("{what} {}", t.meta.id));
            Ok(())
        }
        Command::Attach { id, files } => attach(&ctx(&cli)?, id, files),
        Command::Sync(a) => sync_cmd(&ctx(&cli)?, a),
        Command::Project => project(&ctx(&cli)?),
    }
}

fn print_task(c: &Ctx, t: &task::Task, human: &str) {
    if c.json {
        print_json(&task_json(&c.project, t));
    } else {
        println!("{human}");
    }
}

fn read_details(inline: &Option<String>, file: &Option<String>) -> Result<Option<String>> {
    if let Some(d) = inline {
        return Ok(Some(d.clone()));
    }
    match file.as_deref() {
        None => Ok(None),
        Some("-") => {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .map_err(|e| AppError::Validation(format!("stdin: {e}")))?;
            Ok(Some(s))
        }
        Some(p) => std::fs::read_to_string(p)
            .map(Some)
            .map_err(|e| AppError::NotFound(format!("{p}: {e}"))),
    }
}

fn create(c: &Ctx, a: &CreateArgs) -> Result<()> {
    let details = read_details(&a.details, &a.details_file)?.unwrap_or_default();
    let t = task::create_task(
        &c.store,
        &c.project,
        &c.cfg,
        NewTask {
            title: a.title.clone(),
            description: a.description.clone().unwrap_or_default(),
            details,
            priority: a.priority,
            status: a.status.clone(),
            tags: a.tags.clone(),
            pinned: a.pin,
            ..Default::default()
        },
    )?;
    print_task(c, &t, &format!("created {}: {}", t.meta.id, t.meta.title));
    Ok(())
}

fn list(cli: &Cli, a: &ListArgs) -> Result<()> {
    let store;
    let projects: Vec<(Project, Config)>;
    let current: Option<Project>;
    if a.all_projects {
        store = Store::resolve();
        let mut v = Vec::new();
        for p in store.registered_projects()? {
            let cfg = Config::load(&store.root, Some(&p.folder))?;
            v.push((p, cfg));
        }
        projects = v;
        current = None;
    } else {
        let c = ctx(cli)?;
        current = Some(c.project.clone());
        projects = vec![(c.project, c.cfg)];
    }
    let mut tasks = Vec::new();
    let mut errors = Vec::new();
    let mut lines = Vec::new();
    for (p, cfg) in &projects {
        if a.all_projects {
            lines.push(format!("# {}", p.key.display()));
        }
        for e in task::load_tasks(p, cfg) {
            match &e {
                TaskEntry::Ok(t) => {
                    let m = &t.meta;
                    if a.status.as_ref().is_some_and(|s| s != &m.status)
                        || a.priority.is_some_and(|pr| pr != m.priority)
                        || a.tag.as_ref().is_some_and(|tg| !m.tags.contains(tg))
                    {
                        continue;
                    }
                    lines.push(task_line(t));
                    tasks.push(task_json(p, t));
                }
                TaskEntry::Broken(b) => {
                    lines.push(format!("{} ! {}: {}", b.id, b.path.display(), b.error));
                    errors.extend(output::broken_json(&e));
                }
            }
        }
    }
    if cli.json {
        print_json(&json!({
            "project": current.as_ref().map(project_json),
            "tasks": tasks,
            "errors": errors,
        }));
    } else if lines.is_empty() {
        println!("no tasks");
    } else {
        println!("{}", lines.join("\n"));
    }
    Ok(())
}

fn show(c: &Ctx, id: &str, with_details: bool) -> Result<()> {
    let t = task::load_task(&c.project, &c.cfg, id)?;
    let details = with_details.then(|| t.details());
    if c.json {
        let mut v = task_json(&c.project, &t);
        if let Some(d) = details {
            v["details"] = json!(d);
        }
        print_json(&v);
    } else {
        print!("{}", task_block(&t, details.as_deref()));
    }
    Ok(())
}

fn update(c: &Ctx, a: &UpdateArgs) -> Result<()> {
    let patch = TaskPatch {
        title: a.title.clone(),
        description: a.description.clone(),
        details: read_details(&a.details, &a.details_file)?,
        append_details: a.append_details.clone(),
        priority: a.priority,
        status: a.status.clone(),
        pinned: None,
        add_tags: a.tags.clone(),
        remove_tags: a.untags.clone(),
    };
    let t = task::update_task(&c.project, &c.cfg, &a.id, patch)?;
    print_task(c, &t, &format!("updated {}", t.meta.id));
    Ok(())
}

fn attach(c: &Ctx, id: &str, files: &[PathBuf]) -> Result<()> {
    let added = task::attach_files(&c.project, &c.cfg, id, files)?;
    let id = crate::store::ids::normalize_id(id)?;
    if c.json {
        print_json(&json!({
            "task": id,
            "attachments": added.iter().map(|a| json!({
                "name": a.name, "size": a.size, "path": a.path.display().to_string()
            })).collect::<Vec<_>>(),
        }));
    } else {
        for a in &added {
            println!("attached {} to {id}", a.name);
        }
    }
    Ok(())
}

fn project(c: &Ctx) -> Result<()> {
    let mut v = project_json(&c.project);
    v["root"] = json!(c.store.root.display().to_string());
    v["root_source"] = json!(c.store.root_source.as_str());
    v["config"] = serde_json::to_value(&c.cfg).unwrap_or_default();
    if c.json {
        print_json(&v);
    } else {
        println!("project: {}", c.project.key.display());
        println!("folder:  {}", c.project.dir.display());
        println!(
            "root:    {} ({})",
            c.store.root.display(),
            c.store.root_source.as_str()
        );
        println!("sync:    {}", c.cfg.github.mode.as_str());
    }
    Ok(())
}

fn sync_cmd(c: &Ctx, a: &SyncArgs) -> Result<()> {
    let mode = c.cfg.github.mode;
    let cfg_path = config::project_path(&c.store.root, &c.project.folder);
    let flag = if a.list_new {
        Some("--list-new")
    } else if !a.import.is_empty() {
        Some("--import")
    } else if !a.ignore.is_empty() {
        Some("--ignore")
    } else if !a.unignore.is_empty() {
        Some("--unignore")
    } else {
        None
    };
    if let Some(f) = flag {
        if !mode.pulls() {
            return Err(AppError::Validation(format!(
                "`sync {f}` needs github.mode = \"pull\" or \"two-way\"; mode is `{}` for {} (set it in {})",
                mode.as_str(),
                c.project.key.display(),
                cfg_path.display()
            )));
        }
    }
    if !a.ignore.is_empty() || !a.unignore.is_empty() {
        let st = sync::state::edit(&c.project, &a.ignore, &a.unignore)?;
        if c.json {
            print_json(&json!({ "ignored_issues": st.ignored_issues }));
        } else {
            println!("ignored issues: {:?}", st.ignored_issues);
        }
        return Ok(());
    }
    if mode == SyncMode::Off {
        if c.json {
            print_json(&json!({"mode": "off", "skipped": true}));
        } else {
            println!(
                "GitHub sync is off for {} (set github.mode in {})",
                c.project.key.display(),
                cfg_path.display()
            );
        }
        return Ok(());
    }
    let session = Session::connect(&c.project, &c.cfg, &c.store.root)?;
    for w in &session.warnings {
        eprintln!("warning: {w}");
    }
    let fetched = sync::fetch(&session, &c.project, &c.cfg)?;
    if a.list_new {
        let st = sync::state::load(&c.project)?;
        let cands = fetched.candidates(&st.ignored_issues);
        if c.json {
            print_json(&json!({
                "repo": session.repo,
                "client": session.api.kind(),
                "candidates": cands.iter().map(|i| json!({
                    "number": i.number, "title": i.title, "labels": i.labels,
                    "updated_at": i.updated_at.to_rfc3339(),
                })).collect::<Vec<_>>(),
                "ignored": st.ignored_issues,
            }));
        } else if cands.is_empty() {
            println!("no new issues in {}", session.repo);
        } else {
            for i in &cands {
                println!("#{}  {}", i.number, i.title);
            }
        }
        return Ok(());
    }
    if !a.import.is_empty() {
        let opts = RunOptions {
            selection: Selection::Only(a.import.clone()),
            ignore_add: Vec::new(),
            dry_run: false,
            imports_only: true,
        };
        let r = sync::run_fetched(&c.store, &c.project, &c.cfg, &session, fetched, &opts)?;
        if r.imported_links.is_empty() {
            return Err(AppError::NotFound(format!(
                "nothing imported: {}",
                r.skipped.join("; ")
            )));
        }
        if c.json {
            print_json(&json!({
                "imported": r.imported_links.iter().map(|(id, n)| json!({"id": id, "issue": n})).collect::<Vec<_>>(),
                "skipped": r.skipped,
            }));
        } else {
            for (id, n) in &r.imported_links {
                println!("imported #{n} as {id}");
            }
            for s in &r.skipped {
                println!("skipped {s}");
            }
        }
        return Ok(());
    }
    let r = sync::run_fetched(
        &c.store,
        &c.project,
        &c.cfg,
        &session,
        fetched,
        &RunOptions::all(a.dry_run),
    )?;
    if c.json {
        print_json(&serde_json::to_value(&r).unwrap_or_default());
    } else {
        println!("{}", summary(&r));
    }
    Ok(())
}

pub fn summary(r: &sync::SyncReport) -> String {
    let mut s = format!(
        "{}{} {} via {}: imported {}, updated {}, pushed {}, conflicts {}",
        if r.dry_run { "[dry-run] " } else { "" },
        r.mode,
        r.repo,
        r.client,
        r.imported.len(),
        r.updated_local.len(),
        r.pushed.len(),
        r.conflicts.len()
    );
    for x in &r.skipped {
        s.push_str(&format!("\nskipped: {x}"));
    }
    s
}
