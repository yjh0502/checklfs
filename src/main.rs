use anyhow::Result;
use argh::*;
use git2::*;
use log::*;
use std::path::{Path, PathBuf};

fn iter_tree(repo: &Repository, prefix: &Path, tree: Tree) -> Result<usize> {
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
                count += iter_tree(repo, &prefix, tree)?;
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
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("checklfs=info"),
    )

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
    let error_count = iter_tree(&repo, &root.as_path(), tree)?;

    info!("elapsed={}ms, errors={}", sw.elapsed_ms(), error_count);

    if error_count > 0 {
        std::process::exit(1);
    }

    Ok(())
}
