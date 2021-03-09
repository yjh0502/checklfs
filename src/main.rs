use anyhow::Result;
use argh::*;
use git2::*;
use log::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

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

        match obj.kind() {
            Some(ObjectType::Tree) => {
                let tree = obj.peel_to_tree()?;
                let prefix = prefix.join(&name);
                count += iter_tree_case(repo, &prefix, &tree, names)?;
            }
            Some(ObjectType::Blob) => {
                let name = prefix.join(&name);

                let path_str = name.to_str().expect("non-utf8 filename");
                let lower_path_str = path_str.to_lowercase();

                if !names.insert(lower_path_str) {
                    error!("case-insensitive duplicated entry: {}", path_str);
                    count += 1;
                }
            }
            _ => {
                continue;
            }
        }
    }

    Ok(count)
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

    let sw = stopwatch::Stopwatch::start_new();
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

    let tree = repo.find_tree(commit.tree_id())?;
    let root = PathBuf::new();

    info!("checking case-insensitive-duplicated files");
    let mut name_set = HashSet::new();
    let case_error_count = iter_tree_case(&repo, &root.as_path(), &tree, &mut name_set)?;

    info!("checking invalid lfs files");
    let lfs_error_count = iter_tree_lfs(&repo, &root.as_path(), &tree)?;

    info!(
        "elapsed={}ms, lfs-errors={}, case-errors={}",
        sw.elapsed_ms(),
        lfs_error_count,
        case_error_count
    );

    let error_count = lfs_error_count + case_error_count;
    if error_count > 0 {
        std::process::exit(1);
    }

    Ok(())
}
