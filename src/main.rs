use log::{debug, error, trace, warn};
use path_slash::PathExt;
use walkdir::WalkDir;

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::vec::Vec;

mod magic;

const APP_AUTHOR: &str = "Matt Schulte <schultetwin1@gmail.com>";
const APP_NAME: &str = "sourcelynk";

const ELF_SOURCE_LINK_SECTION_NAME: &str = ".debug_sourcelink";

fn main() -> Result<(), std::io::Error> {
    let matches = parse_cli_args();
    initialize_logger(&matches);

    for entry in WalkDir::new(matches.value_of("PATH").unwrap())
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| is_possible_symbol_file(e))
        .map(|e| e.path().to_owned())
    {
        trace!("Checking {} for embedded sources", entry.display());
        // we've already opened the file once, it should work again
        let file = File::open(&entry).unwrap();

        // get list of embedded source files
        let source_files = match compiledfiles::parse(file) {
            Ok(files) => files,
            Err(e) => match e {
                compiledfiles::Error::MissingDebugSymbols => {
                    debug!("{} is missing debug symbols", entry.display());
                    continue;
                }
                compiledfiles::Error::UnrecognizedFileFormat => {
                    debug!("{} is an unrecognized format", entry.display());
                    continue;
                }
                _ => {
                    warn!(
                        "Unexpected parsing error of known file \"{}\": {}",
                        entry.display(),
                        e
                    );
                    continue;
                }
            },
        };

        if source_files.is_empty() {
            warn!(
                "{} was parsed but contained no source files",
                entry.display()
            );
            continue;
        }

        trace!(
            "{} contains {} source files",
            entry.display(),
            source_files.len()
        );

        // generate source file to path mapping
        let repos = repos_from_source_files(&source_files);
        trace!("Found {} repos for {}", repos.len(), entry.display());
        // generate mapping of directories to urls
        let mapping = generate_mapping(&repos);

        if !mapping.is_empty() {
            let json = serde_json::json!({ "documents": mapping });
            if matches.is_present("dryrun") {
                println!("Would update {}", entry.display());
                println!("{}", serde_json::to_string_pretty(&json).unwrap());
                println!();
            } else {
                let temp_json_file = tempfile::NamedTempFile::new().unwrap();
                let (json_file, json_path) = temp_json_file.keep().unwrap();
                let section_name = ELF_SOURCE_LINK_SECTION_NAME;
                let section_arg = format!("{}={}", section_name, json_path.to_str().unwrap());
                serde_json::to_writer(json_file, &json).unwrap();

                let temp_output_elf_file = tempfile::NamedTempFile::new().unwrap();
                let (_, output_elf_path) = temp_output_elf_file.keep().unwrap();
                let cmd_output = Command::new("objcopy")
                    .arg("--add-section")
                    .arg(section_arg)
                    .arg(entry.to_str().unwrap())
                    .arg(output_elf_path.to_str().unwrap())
                    .output()
                    .unwrap();

                if cmd_output.status.success() {
                    std::fs::rename(output_elf_path, entry.clone()).unwrap();
                    println!(
                        "Updated {}",
                        std::fs::canonicalize(&entry).unwrap().display()
                    );
                } else {
                    println!(
                        "Failed to update {}",
                        std::fs::canonicalize(&entry).unwrap().display()
                    );
                    debug!("{}", std::str::from_utf8(&cmd_output.stderr).unwrap());
                }
            }
        }
    }
    Ok(())
}

fn repos_from_source_files(source_files: &[compiledfiles::FileInfo]) -> Vec<git2::Repository> {
    let mut repos = Vec::<git2::Repository>::new();
    for file in source_files {
        trace!("Searching for repo for {}", file.path.display());
        if file.path.is_file() {
            if let Some(repo) = repo_from_source_file(&file.path) {
                trace!(
                    "Found repo {} for {}",
                    repo.workdir().unwrap().display(),
                    file.path.display()
                );
                let rel_path = file.path.strip_prefix(repo.workdir().unwrap()).unwrap();
                let rel_path = PathBuf::from(rel_path.to_slash().unwrap());
                if repos
                    .iter()
                    .any(|x| x.workdir().unwrap() == repo.workdir().unwrap())
                {
                    // Do nothing, we already know about this repo
                } else if repo
                    .head()
                    .unwrap()
                    .peel_to_tree()
                    .unwrap()
                    .get_path(&rel_path)
                    .is_ok()
                {
                    repos.push(repo);
                } else {
                    debug!(
                        "{} not tracked in git repo {}",
                        file.path.display(),
                        repo.workdir().unwrap().display()
                    );
                }
            }
        } else {
            debug!(
                "Not indexing {} as it does not exists on disk",
                file.path.display()
            );
        }
    }
    repos
}

fn repo_from_source_file(path: &Path) -> Option<git2::Repository> {
    match git2::Repository::discover(path) {
        Ok(repo) => Some(repo),
        Err(e) if e.code() == git2::ErrorCode::NotFound => {
            debug!(
                "Not indexing {} as it is not tracked by source control",
                path.display()
            );
            None
        }
        Err(e) => {
            warn!("Error {} discovering git repo at \"{}\"", e, path.display());
            None
        }
    }
}

fn generate_mapping(repos: &[git2::Repository]) -> HashMap<PathBuf, String> {
    let mut map = HashMap::default();
    for repo in repos {
        let workdir = repo.workdir().unwrap();

        let remote = match repo.find_remote("origin") {
            Ok(remote) => remote,
            Err(e) => {
                match e.code() {
                    git2::ErrorCode::NotFound => {
                        warn!(
                            "Skipping repo {}. No remote named origin",
                            workdir.display()
                        );
                    }
                    _ => {
                        error!(
                            "Skipping repo {}. Unexpected error getting remote {}",
                            workdir.display(),
                            e
                        );
                    }
                };
                continue;
            }
        };

        let remote_url_str = match remote.url() {
            Some(url) => url,
            None => {
                error!("Skiping repo {}. URL is invalid", workdir.display());
                continue;
            }
        };

        let remote_url = match url::Url::parse(remote_url_str) {
            Ok(url) => url,
            Err(e) => {
                warn!(
                    "Skipping repo {}. Unable to parse url due to: {}",
                    workdir.display(),
                    e
                );
                continue;
            }
        };

        let head = repo.head().unwrap();
        let hash = head.target().unwrap();
        match generate_url(&remote_url, &hash) {
            Some(url) => {
                map.insert(workdir.join("*"), url.into());
            }
            None => {
                warn!(
                    "Skipping repo {}. Unable to generate url",
                    workdir.display()
                );
            }
        }
    }
    map
}

fn generate_url(url: &url::Url, hash: &git2::Oid) -> Option<url::Url> {
    if let Some(domain) = url.domain() {
        if domain == "github.com" {
            Some(generate_github_url(url, hash))
        } else if domain.ends_with("visualstudio.com") {
            Some(generate_azure_devops_url(url, hash))
        } else {
            warn!("{} is not a known domain ({})", domain, url);
            None
        }
    } else {
        warn!("Url {} has no domain", url);
        None
    }
}

fn generate_github_url(url: &url::Url, hash: &git2::Oid) -> url::Url {
    let components = url.path_segments().unwrap().collect::<Vec<&str>>();

    let user = components[0];
    let repo = components[1];

    let url_str = format!(
        "https://api.github.com/repos/{}/{}/contents/*?ref={}",
        user, repo, hash
    );

    url::Url::parse(&url_str).unwrap()
}

fn generate_azure_devops_url(url: &url::Url, hash: &git2::Oid) -> url::Url {
    let components = url.path_segments().unwrap().collect::<Vec<&str>>();
    let domain = url.domain().unwrap();

    let organization = domain.split('.').next().unwrap();
    let project = components[1];
    let repo = components[3];
    let url_str = format!(
        "https://dev.azure.com/{}/{}/_apis/git/repositories/{}/items?versionDescriptor.versionType=commit&versionDescriptor.version={}&api-version=5.1&path=/*",
        organization,
        project,
        repo,
        hash
    );

    url::Url::parse(&url_str).unwrap()
}

fn initialize_logger(matches: &clap::ArgMatches) {
    // Vary the output based on how many times the user used the "verbose" flag
    // (i.e. 'myprog -v -v -v' or 'myprog -vvv' vs 'myprog -v'
    let mut logger = pretty_env_logger::formatted_builder();
    let logger = match matches.occurrences_of("v") {
        0 => logger.filter_level(log::LevelFilter::Error),
        1 => logger.filter_level(log::LevelFilter::Warn),
        2 => logger.filter_level(log::LevelFilter::Info),
        3 => logger.filter_level(log::LevelFilter::Debug),
        _ => logger.filter_level(log::LevelFilter::Trace),
    };
    logger.init();
    trace!("logger initialized");
}

fn is_possible_symbol_file(entry: &walkdir::DirEntry) -> bool {
    let path = entry.path();
    match File::open(path) {
        Ok(ref mut file) => match magic::file_type(file).unwrap_or(magic::FileType::Unknown) {
            magic::FileType::Elf(magic::ElfType::Exec)
            | magic::FileType::Elf(magic::ElfType::Dyn)
            | magic::FileType::Pdb => true,

            magic::FileType::Elf(magic::ElfType::None)
            | magic::FileType::Elf(magic::ElfType::Core)
            | magic::FileType::Elf(magic::ElfType::Rel)
            | magic::FileType::Elf(magic::ElfType::Unknown)
            | magic::FileType::MachO
            | magic::FileType::PE
            | magic::FileType::Unknown => {
                trace!("File type not usabled for {}", entry.path().display());
                false
            }
        },
        Err(e) => {
            warn!("Failed to open {} due to {}", path.display(), e);
            false
        }
    }
}

fn parse_cli_args<'a>() -> clap::ArgMatches<'a> {
    clap::App::new(APP_NAME)
        .version(env!("CARGO_PKG_VERSION"))
        .about("CLI tool for dbgsrv")
        .author(APP_AUTHOR)
        .arg(
            clap::Arg::with_name("v")
                .short("v")
                .multiple(true)
                .help("Sets the level of verbosity"),
        )
        .arg(
            clap::Arg::with_name("dryrun")
                .short("n")
                .long("dryrun")
                .help("Run without modifying the binaries"),
        )
        .arg(
            clap::Arg::with_name("PATH")
                .help("Path to search for debug info files")
                .default_value(".")
                .index(1),
        )
        .get_matches()
}
