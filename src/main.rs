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

fn test_meta<P: AsRef<Path>>(repo_root: P, commit_id: &str) -> Result<usize> {
    info!("checking meta files");

    let repo = git2::Repository::open(repo_root)?;
    let commit_id = git2::Oid::from_str(commit_id)?;
    let tree = repo.find_tree(commit_id)?;
    let root = PathBuf::new();

    let mut count = 0;
    let mut name_set = HashMap::new();
    iter_tree_meta(&repo, &root.as_path(), &tree, &mut name_set)?;

    for (path, status) in name_set {
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
    Ok(count)
}

fn iter_tree_meta(
    repo: &Repository,
    prefix: &Path,
    tree: &Tree,
    names: &mut HashMap<PathBuf, MetaStatus>,
) -> Result<()> {
    for entry in tree.iter() {
        let name = match entry.name() {
            None => continue,
            Some(name) => name,
        };

        // Unity3d ignores following files
        if name.starts_with(".") || name.ends_with("~") {
            continue;
        }

        let obj = entry.to_object(repo)?;
        let name = prefix.join(&name);

        match obj.kind() {
            Some(ObjectType::Tree) => {
                let tree = obj.peel_to_tree()?;

                iter_tree_meta(repo, &name, &tree, names)?;

                if let Some(v) = names.get_mut(&name) {
                    v.file = true;
                } else {
                    names.insert(name, MetaStatus::file());
                }
            }
            Some(ObjectType::Blob) => {
                let (base_path, is_meta) = if let Some(ext) = name.extension() {
                    if ext == "meta" {
                        let mut name = name.to_owned();
                        name.set_extension("");
                        (name, true)
                    } else {
                        (name.to_owned(), false)
                    }
                } else {
                    (name.to_owned(), false)
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
            }
            _ => {
                continue;
            }
        }
    }

    Ok(())
}

fn test_case<P: AsRef<Path>>(repo_root: P, commit_id: &str) -> Result<usize> {
    info!("checking case-insensitive-duplicated files");

    let repo = git2::Repository::open(repo_root)?;
    let commit_id = git2::Oid::from_str(commit_id)?;
    let tree = repo.find_tree(commit_id)?;
    let root = PathBuf::new();

    let mut name_set = HashSet::new();
    iter_tree_case(&repo, &root.as_path(), &tree, &mut name_set)
}

fn iter_tree_case(
    repo: &Repository,
    prefix: &Path,
    tree: &Tree,
    names: &mut HashSet<String>,
) -> Result<usize> {
    let mut count = 0;

    for entry in tree.iter() {
        let name = match entry.name() {
            None => continue,
            Some(name) => name,
        };
        let obj = entry.to_object(repo)?;
        let name = prefix.join(&name);

        let path_str = name.to_str().expect("non-utf8 filename");
        let lower_path_str = path_str.to_lowercase();

        if !names.insert(lower_path_str) {
            error!("case-insensitive duplicated entry: {}", path_str);
            count += 1;
        }

        if let Some(ObjectType::Tree) = obj.kind() {
            let tree = obj.peel_to_tree()?;
            count += iter_tree_case(repo, &name, &tree, names)?;
        }
    }

    Ok(count)
}

fn test_lfs<P: AsRef<Path>>(repo_root: P, commit_id: &str) -> Result<usize> {
    info!("checking invalid lfs files");

    let repo = git2::Repository::open(repo_root)?;
    let commit_id = git2::Oid::from_str(commit_id)?;
    let tree = repo.find_tree(commit_id)?;
    let root = PathBuf::new();

    iter_tree_lfs(&repo, &root, &tree)
}

fn iter_tree_lfs(repo: &Repository, prefix: &Path, tree: &Tree) -> Result<usize> {
    let mut count = 0;

    for entry in tree.iter() {
        let name = match entry.name() {
            None => continue,
            Some(name) => name,
        };

        let obj = entry.to_object(repo)?;

        match obj.kind() {
            Some(ObjectType::Tree) => {
                let tree = obj.peel_to_tree()?;
                let prefix = prefix.join(&name);
                count += iter_tree_lfs(repo, &prefix, &tree)?;
            }
            Some(ObjectType::Blob) => {
                let blob = obj.peel_to_blob()?;
                let size = blob.size();

                let full_path = Path::join(prefix, name);

                let attr = repo.get_attr(&full_path, "merge", git2::AttrCheckFlags::INDEX_ONLY)?;
                if attr != Some("lfs") {
                    continue;
                }

                // TODO: check content
                if size < 150 {
                    continue;
                }
                error!("should be in LFS: {:?}, {}", full_path, size);
                count += 1;
            }
            _ => {
                continue;
            }
        }
    }
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
