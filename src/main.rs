use anyhow::Result;
use argh::*;
use git2::*;
use log::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Default)]
struct MetaStatus {
    file: bool,
    meta: bool,
}

impl MetaStatus {
    fn file() -> Self {
        MetaStatus {
            file: true,
            meta: false,
        }
    }
    fn meta() -> Self {
        MetaStatus {
            file: false,
            meta: true,
        }
    }
}

fn test_meta_ignore(name: &str) -> bool {
    // Unity3d ignores following files
    name.starts_with(".") || name.ends_with("~")
}

fn test_meta<P: AsRef<Path>>(repo_root: P, commit_id: &str) -> Result<usize> {
    info!("checking meta files");

    let repo = git2::Repository::open(repo_root)?;
    let commit_id = git2::Oid::from_str(commit_id)?;
    let tree = repo.find_tree(commit_id)?;

    let mut count = 0;
    let mut names = HashMap::<PathBuf, MetaStatus>::new();

    tree.walk(TreeWalkMode::PreOrder, |base_path, entry| {
        let name = match entry.name() {
            None => return TreeWalkResult::Ok,
            Some(name) => name,
        };
        let full_path = Path::new(base_path).join(name);
        let path_str = full_path.to_str().unwrap();

        if test_meta_ignore(name) {
            return TreeWalkResult::Skip;
        }

        let (base_path, is_meta) = if name == "" {
            let path_str = &path_str[0..path_str.len() - 1];
            (Path::new(path_str).to_owned(), false)
        } else if let Some(ext) = full_path.extension() {
            if ext == "meta" {
                let mut full_path = full_path.to_owned();
                full_path.set_extension("");
                (full_path, true)
            } else {
                (full_path.to_owned(), false)
            }
        } else {
            (full_path.to_owned(), false)
        };

        if let Some(v) = names.get_mut(&base_path) {
            if is_meta {
                v.meta = true;
            } else {
                v.file = true;
            }
        } else {
            if is_meta {
                names.insert(base_path, MetaStatus::meta());
            } else {
                names.insert(base_path, MetaStatus::file());
            }
        }

        TreeWalkResult::Ok
    })?;

    for (path, status) in names {
        if !path.starts_with("Assets/") || (path == Path::new("Assets")) {
            continue;
        }

        if !(status.file && status.meta) {
            count += 1;
            error!(
                "invalid status: path={:?}, file={}, meta={}",
                path, status.file, status.meta
            );
        }
    }

    info!("checking meta files: done");
    Ok(count)
}

fn test_case<P: AsRef<Path>>(repo_root: P, commit_id: &str) -> Result<usize> {
    info!("checking case-insensitive-duplicated files");

    let repo = git2::Repository::open(repo_root)?;
    let commit_id = git2::Oid::from_str(commit_id)?;
    let tree = repo.find_tree(commit_id)?;

    let mut names = HashSet::new();
    let mut count = 0;

    tree.walk(TreeWalkMode::PreOrder, |base_path, entry| {
        let name = match entry.name() {
            None => return TreeWalkResult::Ok,
            Some(name) => name,
        };
        let full_path = Path::new(base_path).join(name);
        let path_str = full_path.to_str().unwrap();

        let lower_path_str = path_str.to_lowercase();
        if !names.insert(lower_path_str) {
            error!("case-insensitive duplicated entry: {}", path_str);
            count += 1;
        }

        TreeWalkResult::Ok
    })?;

    info!("checking case-insensitive-duplicated files: done");
    Ok(count)
}

fn test_lfs_walk<'a>(
    repo: &'a Repository,
    base_path: &'a str,
    entry: &'a TreeEntry<'a>,
) -> Result<usize> {
    let name = match entry.name() {
        Some(name) => name,
        None => return Ok(0),
    };

    let full_path = Path::new(base_path).join(name);
    let attr = repo.get_attr(&full_path, "merge", git2::AttrCheckFlags::INDEX_ONLY)?;
    if attr != Some("lfs") {
        return Ok(0);
    }

    let obj = entry.to_object(repo)?;
    if let Some(ObjectType::Blob) = obj.kind() {
        let blob = obj.peel_to_blob()?;
        let size = blob.size();

        // TODO: check content
        if size < 150 {
            return Ok(0);
        }
        error!("should be in LFS: {:?}, {}", full_path, size);
        Ok(1)
    } else {
        Ok(0)
    }
}

fn test_lfs<P: AsRef<Path>>(repo_root: P, commit_id: &str) -> Result<usize> {
    info!("checking invalid lfs files");

    let repo = git2::Repository::open(repo_root)?;
    let commit_id = git2::Oid::from_str(commit_id)?;
    let tree = repo.find_tree(commit_id)?;
    let mut count = 0;

    tree.walk(TreeWalkMode::PreOrder, |base_path, entry| {
        if let Ok(num) = test_lfs_walk(&repo, base_path, entry) {
            count += num;
        }
        TreeWalkResult::Ok
    })?;

    info!("checking invalid lfs files: done");
    Ok(count)
}

#[derive(FromArgs, Debug)]
#[argh(description = "checklfs")]
struct CommandRoot {
    #[argh(positional)]
    path: String,

    #[argh(option, description = "commit")]
    commit: Option<String>,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    git2::opts::enable_caching(true);

    let arg: CommandRoot = argh::from_env();

    info!("repository={}", arg.path);

    let start = Instant::now();
    let repo = git2::Repository::open(&arg.path)?;

    let commit = match arg.commit {
        Some(commit) => {
            let oid = Oid::from_str(&commit)?;
            let commit = repo.find_commit(oid)?;
            commit
        }
        None => {
            let head = repo.head()?.resolve()?.target().unwrap();
            let commit = repo.find_commit(head)?;
            commit
        }
    };

    let commit_id = commit.tree_id().to_string();
    let path = arg.path.to_string();

    let path0 = path.clone();
    let commit_id0 = commit_id.clone();
    let t_meta = std::thread::spawn(move || test_meta(&path0, &commit_id0));

    let path0 = path.clone();
    let commit_id0 = commit_id.clone();
    let t_case = std::thread::spawn(move || test_case(&path0, &commit_id0));

    let path0 = path.clone();
    let commit_id0 = commit_id.clone();
    let t_lfs = std::thread::spawn(move || test_lfs(&path0, &commit_id0));

    let meta_error_count = t_meta.join().unwrap()?;
    let case_error_count = t_case.join().unwrap()?;
    let lfs_error_count = t_lfs.join().unwrap()?;

    info!(
        "elapsed={:?}, meta-errors={}, lfs-errors={}, case-errors={}",
        start.elapsed(),
        meta_error_count,
        lfs_error_count,
        case_error_count
    );

    let error_count = meta_error_count + case_error_count + lfs_error_count;
    if error_count > 0 {
        std::process::exit(1);
    }

    Ok(())
}
