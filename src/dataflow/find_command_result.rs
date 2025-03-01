use crate::config::Config;
use crate::data::lock::lockfile::{Lockfile, LockfileError};
use crate::data::manifest::Manifest;
use crate::dataflow::lockfile_packages::LockfileResult;
use crate::dataflow::manifest_packages::ManifestResult;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::graphql::execute_query;
use graphql_client::*;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "graphql/schema.graphql",
    query_path = "graphql/queries/get_package_by_command.graphql",
    response_derives = "Debug"
)]
struct GetPackageByCommandQuery;

#[derive(Debug)]
pub struct PackageInfoFromCommand {
    pub command: String,
    pub version: String,
    pub namespaced_package_name: String,
}

impl PackageInfoFromCommand {
    fn get_response(
        command_name: String,
    ) -> anyhow::Result<get_package_by_command_query::ResponseData> {
        let q = GetPackageByCommandQuery::build_query(get_package_by_command_query::Variables {
            command_name,
        });
        execute_query(&q)
    }

    pub fn get(command_name: String) -> anyhow::Result<Self> {
        let response = Self::get_response(command_name)?;
        let response_val = response
            .get_command
            .ok_or_else(|| anyhow!("Error getting packages for given command from server"))?;
        Ok(Self {
            command: response_val.command,
            version: response_val.package_version.version,
            namespaced_package_name: response_val.package_version.package.display_name,
        })
    }
}

#[derive(Clone, Debug, Error)]
pub enum Error {
    #[error(
        "Command \"{0}\" was not found in the local directory or the global install directory."
    )]
    CommandNotFound(String),
    #[error(
        "Command \"{0}\" was not found in the local directory. There was an error parsing the global lockfile. {1}",
    )]
    CommandNotFoundInLocalDirectoryAndErrorReadingGlobalDirectory(String, String),
    #[error(
        "Could not get command \"{0}\" because there was a problem with the local package. {1}"
    )]
    ErrorReadingLocalDirectory(String, String),
    #[error(
        "Command \"{0}\" exists in lockfile, but corresponding module \"{}\" not found in lockfile.",
    )]
    CommandFoundButCorrespondingModuleIsMissing(String, String),
    #[error(
        "Failed to get command \"{0}\" because there was an error opening the global installation directory. {}",
    )]
    CouldNotOpenGlobalsDirectory(String, String),
}

#[derive(Debug)]
pub enum FindCommandResult {
    CommandNotFound(String),
    CommandFound {
        source: PathBuf,
        manifest_dir: PathBuf,
        args: Option<String>,
        module_name: String,
        prehashed_cache_key: Option<String>,
    },
    Error(anyhow::Error),
}

impl From<LockfileError> for FindCommandResult {
    fn from(error: LockfileError) -> Self {
        match error {
            LockfileError::CommandNotFound(c) => FindCommandResult::CommandNotFound(c),
            _ => FindCommandResult::Error(error.into()),
        }
    }
}

impl FindCommandResult {
    fn find_command_in_manifest_and_lockfile<S: AsRef<str>>(
        command_name: S,
        manifest: Manifest,
        lockfile: Lockfile,
        directory: &Path,
    ) -> Self {
        match lockfile.get_command(command_name.as_ref()) {
            Err(e) => e.into(),
            Ok(lockfile_command) => {
                debug!("Command found in lockfile: {:?}", &lockfile_command);
                if lockfile_command.package_name == manifest.package.name {
                    // this is a local module command
                    let found_module = manifest.module.as_ref().and_then(|modules| {
                        modules.iter().find(|m| m.name == lockfile_command.module)
                    });
                    match found_module {
                        Some(module) => FindCommandResult::CommandFound {
                            source: module.source.clone(),
                            manifest_dir: manifest.base_directory_path,
                            args: lockfile_command.main_args.clone(),
                            module_name: module.name.clone(),
                            // don't use prehashed cache key for local modules
                            prehashed_cache_key: None,
                        },
                        None => FindCommandResult::Error(
                            Error::CommandFoundButCorrespondingModuleIsMissing(
                                command_name.as_ref().to_string(),
                                lockfile_command.module.clone(),
                            )
                            .into(),
                        ),
                    }
                } else {
                    // this is a module being run as a dependency in a local context
                    debug!(
                        "Command's package name({}) and manifest's package name({}) are different",
                        lockfile_command.package_name, manifest.package.name
                    );
                    match lockfile.get_module(
                        &lockfile_command.package_name,
                        &lockfile_command.package_version,
                        &lockfile_command.module,
                    ) {
                        Ok(lockfile_module) => {
                            let path = lockfile_module
                                .get_canonical_source_path_from_lockfile_dir(directory.into());
                            let manifest_dir = lockfile_module
                                .get_canonical_manifest_path_from_lockfile_dir(
                                    directory.into(),
                                    true,
                                );
                            FindCommandResult::CommandFound {
                                source: path,
                                manifest_dir,
                                args: lockfile_command.main_args.clone(),
                                module_name: lockfile_module.name.clone(),
                                prehashed_cache_key: lockfile
                                    .get_prehashed_cache_key_from_command(&lockfile_command),
                            }
                        }
                        Err(e) => FindCommandResult::Error(e),
                    }
                }
            }
        }
    }

    fn find_command_in_lockfile<S: AsRef<str>>(
        command_name: S,
        lockfile: Lockfile,
        directory: &Path,
    ) -> Self {
        match lockfile.get_command(command_name.as_ref()) {
            Ok(lockfile_command) => {
                match lockfile.get_module(
                    &lockfile_command.package_name,
                    &lockfile_command.package_version,
                    &lockfile_command.module,
                ) {
                    Ok(lockfile_module) => {
                        let path = lockfile_module
                            .get_canonical_source_path_from_lockfile_dir(directory.into());
                        let manifest_dir = lockfile_module
                            .get_canonical_manifest_path_from_lockfile_dir(directory.into(), true);
                        FindCommandResult::CommandFound {
                            source: path,
                            manifest_dir,
                            args: lockfile_command.main_args.clone(),
                            module_name: lockfile_module.name.clone(),
                            prehashed_cache_key: lockfile
                                .get_prehashed_cache_key_from_command(&lockfile_command),
                        }
                    }
                    Err(_e) => {
                        FindCommandResult::CommandNotFound(command_name.as_ref().to_string())
                    }
                }
            }
            Err(_e) => FindCommandResult::CommandNotFound(command_name.as_ref().to_string()),
        }
    }

    pub fn find_command_in_directory<S: AsRef<str>>(directory: &Path, command_name: S) -> Self {
        let manifest_result = ManifestResult::find_in_directory(&directory);
        let lockfile_result = LockfileResult::find_in_directory(&directory);
        match (manifest_result, lockfile_result) {
            (ManifestResult::ManifestError(e), _) => return FindCommandResult::Error(e.into()),
            (_, LockfileResult::LockfileError(e)) => return FindCommandResult::Error(e.into()),
            (ManifestResult::NoManifest, LockfileResult::NoLockfile) => {} // continue
            (ManifestResult::NoManifest, LockfileResult::Lockfile(l)) => {
                debug!("Looking for local command in the lockfile");
                return Self::find_command_in_lockfile(command_name, l, directory);
            }
            // the edge case of a manifest, but no lockfile would an invalid state. This function
            // should always be run after updating the lockfile with the latest manifest changes.
            // If that function were to fail so horribly that it did not error, and no lockfile was
            // generated, then we will get this panic.
            (ManifestResult::Manifest(_m), LockfileResult::NoLockfile) => {
                panic!("Manifest exists, but lockfile not found!")
            }
            (ManifestResult::Manifest(m), LockfileResult::Lockfile(l)) => {
                debug!("Looking for local command in the manifest and lockfile");
                return Self::find_command_in_manifest_and_lockfile(command_name, m, l, directory);
            }
        };
        FindCommandResult::CommandNotFound(command_name.as_ref().to_string())
    }
}

#[derive(Debug)]
pub struct Command {
    // PathBuf, Option<String>, String, bool
    pub source: PathBuf,
    pub manifest_dir: PathBuf,
    pub args: Option<String>,
    pub module_name: String,
    /// whether the command was found in the global context
    pub is_global: bool,
    /// the prehashed module key
    pub prehashed_cache_key: Option<String>,
}

/// Get a command from anywhere, where anywhere is the set of packages in the local lockfile and the global lockfile.
/// A flag indicating global run is also returned. Commands are found in local lockfile first.
pub fn get_command_from_anywhere<S: AsRef<str>>(command_name: S) -> Result<Command, Error> {
    // look in the local directory, update if necessary
    let current_directory = crate::config::Config::get_current_dir().unwrap();
    let local_command_result =
        FindCommandResult::find_command_in_directory(&current_directory, &command_name);

    match local_command_result {
        FindCommandResult::CommandNotFound(_cmd) => {} // continue
        FindCommandResult::CommandFound {
            source,
            manifest_dir,
            args,
            module_name,
            prehashed_cache_key,
        } => {
            return Ok(Command {
                source,
                manifest_dir,
                args,
                module_name,
                is_global: false,
                prehashed_cache_key,
            });
        }
        FindCommandResult::Error(e) => {
            return Err(Error::ErrorReadingLocalDirectory(
                command_name.as_ref().to_string(),
                e.to_string(),
            ));
        }
    };
    trace!("Local command not found");

    // look in the global directory
    let global_directory = Config::get_globals_directory().map_err(|e| {
        Error::CouldNotOpenGlobalsDirectory(command_name.as_ref().to_string(), e.to_string())
    })?;
    let global_command_result =
        FindCommandResult::find_command_in_directory(&global_directory, &command_name);

    match global_command_result {
        FindCommandResult::CommandNotFound(_) => {} // continue
        FindCommandResult::CommandFound {
            source,
            manifest_dir,
            args,
            module_name,
            prehashed_cache_key,
        } => {
            return Ok(Command {
                source,
                manifest_dir,
                args,
                module_name,
                is_global: true,
                prehashed_cache_key,
            });
        }
        FindCommandResult::Error(e) => {
            return Err(
                Error::CommandNotFoundInLocalDirectoryAndErrorReadingGlobalDirectory(
                    command_name.as_ref().to_string(),
                    e.to_string(),
                ),
            );
        }
    };
    trace!("Global command not found");

    return Err(Error::CommandNotFound(command_name.as_ref().to_string()));
}
