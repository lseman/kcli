use anyhow::{Context, Result};
use chrono::Utc;
use colored::*;
use dialoguer::{theme::ColorfulTheme, Input, Select};
use dirs_next::config_dir;
use futures::TryStreamExt;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::fs::{self, File};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::process::Command as AsyncCommand;
use walkdir::WalkDir; // Add this import

pub async fn menu_install_kernel(theme: &ColorfulTheme) -> Result<()> {
    let mut config_path = config_dir().unwrap();
    config_path.push("kcli");
    let packages_dir = config_path.join("ksrc");

    let packages = list_kernel_packages(&packages_dir).await?;

    let selection = Select::with_theme(theme)
        .with_prompt("Select a kernel package to install")
        .items(&packages)
        .default(0)
        .interact()?;

    let selected_package = &packages[selection];
    let kernel_src_dir = packages_dir.join(selected_package);
    let pkg_dir = config_path.join("pkg");

    installing_kernel(&kernel_src_dir, &pkg_dir, selected_package).await?;
    println!("Kernel '{}' installed successfully.", selected_package);

    Ok(())
}

pub async fn list_kernel_packages(packages_dir: &Path) -> Result<Vec<String>> {
    let mut packages = Vec::new();
    let mut dir_entries = fs::read_dir(packages_dir)
        .await
        .context("Failed to read packages directory")?;

    while let Some(entry) = dir_entries
        .next_entry()
        .await
        .context("Failed to read directory entry")?
    {
        if let Ok(file_type) = entry.file_type().await {
            if file_type.is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    packages.push(name.to_owned());
                }
            }
        }
    }

    Ok(packages)
}

async fn calculate_directory_size(dir: &Path) -> Result<u64, anyhow::Error> {
    let mut total_size = 0u64;
    let mut dir_entries = fs::read_dir(dir).await?;
    while let Some(entry) = dir_entries.next_entry().await? {
        let path = entry.path();
        if path.is_dir() {
            total_size += Box::pin(calculate_directory_size(&path)).await?;
        } else {
            let metadata = fs::metadata(path).await?;
            total_size += metadata.len();
        }
    }
    Ok(total_size)
}

struct PackageInfo {
    pkgname: &'static str,
    pkgver: &'static str,
    pkgrel: &'static str,
    pkgdesc: &'static str,
    url: &'static str,
    license: &'static str,
    depends: Vec<&'static str>,
    makedepends: Vec<&'static str>,
}

static PACKAGE_INFO: Lazy<PackageInfo> = Lazy::new(|| PackageInfo {
    pkgname: "capykernel",
    pkgver: "0.0.1",
    pkgrel: "1",
    pkgdesc: "Custom kernel package for capykernel",
    url: "https://cachyos.org",
    license: "GPL",
    depends: vec![],
    makedepends: vec![],
});

async fn create_pkginfo_file(install_target: &Path) -> Result<()> {
    // Determine system architecture
    let output = Command::new("uname").arg("-m").output().await?;
    let arch = String::from_utf8(output.stdout)?.trim().to_string();

    // Get the current timestamp
    let builddate = Utc::now().timestamp();

    // Calculate the directory size
    let output = Command::new("du")
        .arg("-sb")
        .arg("--apparent-size")
        .arg(install_target.to_str().unwrap()) // Ensure the path is correctly converted to string
        .output()
        .await?;
    let size = String::from_utf8(output.stdout)?
        .split_whitespace()
        .next()
        .unwrap_or("0")
        .to_string();

    // Prepare the content of the .PKGINFO file
    let pkginfo_content = format!(
        "pkgname = {}\n\
        pkgver = {}-{}\n\
        pkgdesc = {}\n\
        url = {}\n\
        license = {}\n\
        builddate = {}\n\
        size = {}\n\
        arch = {}\n\
        {}",
        PACKAGE_INFO.pkgname,
        PACKAGE_INFO.pkgver,
        PACKAGE_INFO.pkgrel,
        PACKAGE_INFO.pkgdesc,
        PACKAGE_INFO.url,
        PACKAGE_INFO.license,
        builddate,
        size,
        arch,
        PACKAGE_INFO
            .depends
            .iter()
            .map(|d| format!("depend = {}\n", d))
            .collect::<String>()
            + &PACKAGE_INFO
                .makedepends
                .iter()
                .map(|md| format!("makedepend = {}\n", md))
                .collect::<String>()
    );

    // Write the content to the .PKGINFO file
    let pkginfo_path = install_target.join(".PKGINFO");
    let mut file = File::create(&pkginfo_path).await?;
    file.write_all(pkginfo_content.as_bytes()).await?;
    file.flush().await?;

    Ok(())
}

async fn create_buildinfo_file(install_target: &Path) -> Result<()> {
    let buildinfo_content = "buildenv = (distcc color ccache check !sign)\n\
        options = (!strip docs libtool staticlibs emptydirs zipman purge !upx !debug)\n\
        pkgarch = x86_64\n\
        packager = Laio Seman <laio@ieee.org>\n";

    let buildinfo_path = install_target.join(".BUILDINFO");
    let mut file = File::create(&buildinfo_path).await?;
    file.write_all(buildinfo_content.as_bytes()).await?;
    file.flush().await?;
    Ok(())
}

async fn create_mtree_file(install_target: &Path, kernel_name: &str) -> Result<()> {
    // Ensure the directory where the .MTREE will be created exists
    let mtree_path = install_target.join(".MTREE");
    ensure_directory_exists(mtree_path.parent().unwrap()).await?;

    // Construct the command to create the .MTREE file
    // Specifying .PKGINFO first ensures it is processed first in the archive
    let mtree_command = format!(
        "cd {} && fakeroot -- env LANG=C bsdtar -czf .MTREE --format=mtree \
        --options='!all,use-set,type,uid,gid,mode,time,size,md5,sha256,link' .PKGINFO *",
        install_target.to_str().unwrap()
    );

    // Execute the command using a shell to ensure proper handling of wildcards and other shell features
    let output = Command::new("sh")
        .arg("-c")
        .arg(mtree_command)
        .output()
        .await
        .context("Failed to create .MTREE file")?;

    // Check for successful execution and handle errors
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("Failed to create .MTREE file: {}", stderr));
    }

    Ok(())
}

async fn ensure_directory_exists(path: &Path) -> Result<()> {
    if !path.exists() {
        tokio::fs::create_dir_all(path)
            .await
            .context(format!("Failed to create directory: {}", path.display()))?;
    }
    Ok(())
}
pub async fn installing_kernel(
    kernel_src_dir: &Path,
    base_pkg_dir: &Path,
    kernel_name: &str,
) -> Result<()> {
    // Ensure the target directory for the installation is correct
    let install_target = base_pkg_dir.join(kernel_name);
    fs::create_dir_all(&install_target)
        .await
        .context("Creating kernel install target directory failed")?;

    // Running make commands
    run_make_commands(kernel_src_dir, &install_target).await?;

    // Create .srctree file
    let srctree_path = install_target.join(".srctree");
    let mut srctree_file = File::create(&srctree_path)
        .await
        .context("Creating .srctree file failed")?;
    for entry in WalkDir::new(&install_target) {
        let entry = entry.context("Failed to read directory entry")?;
        if entry.path().is_file() {
            let path = entry
                .path()
                .strip_prefix(&install_target)?
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Invalid file path"))?;
            srctree_file.write_all(path.as_bytes()).await?;
            srctree_file.write_all(b"\n").await?;
        }
    }

    // Metadata and packaging
    create_pkginfo_file(&install_target).await?;
    create_buildinfo_file(&install_target).await?;
    create_mtree_file(&install_target, kernel_name).await?; // Adjusted to use install_target

    println!("Kernel dir is {}", kernel_src_dir.display());
    // Copy kernel image
    let bzimage_path = kernel_src_dir.join("arch/x86/boot/bzImage");

    println!("Copying bzImage to: {}", bzimage_path.display());
    let vmlinuz_path = install_target
        .join("boot")
        .join(format!("vmlinuz-capycachy-{}", kernel_name));
    // create the boot directory
    fs::create_dir_all(install_target.join("boot"))
        .await
        .context("Creating boot directory failed")?;
    fs::copy(bzimage_path, vmlinuz_path)
        .await
        .context("Copying bzImage failed")?;

    // Compress the installed kernel directory including .srctree
    compress_kernel_package(&install_target, kernel_name).await?; // Assuming this function exists

    println!(
        "Kernel package '{}' installed and compressed successfully.",
        kernel_name
    );
    Ok(())
}

pub async fn uninstalling_kernel(installed_kernels_dir: &Path, kernel_name: &str) -> Result<()> {
    let installed_kernel_dir = installed_kernels_dir.join(kernel_name);
    fs::remove_dir_all(&installed_kernel_dir)
        .await
        .context("Removing installed kernel package directory failed")?;

    Ok(())
}

pub async fn menu_uninstall_kernel(theme: &ColorfulTheme) -> Result<()> {
    let installed_kernels_dir = Path::new("./pkg/");
    let installed_packages = list_kernel_packages(installed_kernels_dir).await?;

    if installed_packages.is_empty() {
        println!("No installed kernel packages found.");
        return Ok(());
    }

    let selection = Select::with_theme(theme)
        .with_prompt("Select an installed kernel package to uninstall")
        .items(&installed_packages)
        .default(0)
        .interact()?;

    let selected_package = &installed_packages[selection];

    uninstalling_kernel(installed_kernels_dir, selected_package).await?;
    println!(
        "Kernel package '{}' uninstalled successfully.",
        selected_package
    );

    Ok(())
}

async fn run_make_commands(kernel_src_dir: &Path, install_target: &PathBuf) -> Result<()> {
    let config_path = kernel_src_dir.join(".config");

    // Check if the .config file exists
    if !config_path.exists() {
        println!("`.config` file not found, downloading from repository...");
        // URL to download the .config file
        let config_url =
            "https://raw.githubusercontent.com/CachyOS/linux-cachyos/master/linux-cachyos/config";
        let response = reqwest::get(config_url)
            .await
            .context("Failed to download the .config file")?;
        let contents = response
            .text()
            .await
            .context("Failed to read the .config file content")?;

        // Write the contents to the .config file
        tokio::fs::write(&config_path, contents)
            .await
            .context("Failed to write the .config file")?;
        println!(
            "`.config` file downloaded and saved to {}",
            config_path.display()
        );
    } else {
        println!("Using existing `.config` file at {}", config_path.display());
    }

    // Calculate the relative path for the install target for modules
    let install_mod_path = install_target.clone();

    // Running modules_install with INSTALL_MOD_PATH
    println!("Executing `make modules_install`...");
    let status_modules_install = Command::new("make")
        .arg("modules_install")
        .arg(format!(
            "INSTALL_MOD_PATH=../../{}",
            install_mod_path.display()
        ))
        .current_dir(kernel_src_dir)
        .stdout(Stdio::inherit()) // To see the make command output
        .stderr(Stdio::inherit()) // To see the make command errors
        .status()
        .await
        .context("`make modules_install` command failed")?;

    if !status_modules_install.success() {
        return Err(anyhow::anyhow!("`make modules_install` failed"));
    }

    // get kernel name from kernel_src_dir
    //let kernel_name = kernel_src_dir.file_name().unwrap().to_str().unwrap();
    // get only the last part of the kernel name
    //let kernel_name = kernel_name.split("/").last().unwrap();

    //println!("Kernel name: {}", kernel_name);

    // delete build directory
    //let build_dir = "../../name/lib/modules/*/build".replace("name", install_mod_path.to_str().unwrap());
    //println!("Removing build directory: {}", build_dir);
    //fs::remove_dir_all(&build_dir).await.context("Removing build directory failed")?;

    // Construct headers install path (mod_path/build)
    let install_hdr_path = install_mod_path.join("usr");

    println!("Installing modules to: {}", install_mod_path.display());
    println!("Installing headers to: {}", install_hdr_path.display());

    // Running headers_install with INSTALL_HDR_PATH
    /*
    println!("Executing `make headers_install` with INSTALL_HDR_PATH...");
    let status_headers_install = Command::new("make")
        .arg("headers_install")
        .arg(format!(
            "INSTALL_HDR_PATH=../../{}",
            install_hdr_path.display()
        ))
        .current_dir(kernel_src_dir)
        .stdout(Stdio::inherit()) // Inherit stdout to see command output
        .stderr(Stdio::inherit()) // Inherit stderr to see any errors
        .status()
        .await
        .context("`make headers_install` command failed")?;

    if !status_headers_install.success() {
        return Err(anyhow::anyhow!("`make headers_install` failed"));
    }
     */

    Ok(())
}
async fn compress_kernel_package(pkg_dir: &Path, kernel_name: &str) -> Result<()> {
    // Construct the tarball path

    let mut config_path = config_dir().unwrap();
    config_path.push("kcli");
    let packages_dir = config_path.join("pkg");
    let kernel_path = packages_dir.join(kernel_name);
    let current_dir = std::env::current_dir().unwrap();

    // Construct the bsdtar command to respect the order of files
    let bsdtar_command = format!(
        "fakeroot -- env LANG=C sh -c 'cd {} && bsdtar -cf - .MTREE .PKGINFO * | gzip -c > {}/{}.capy.tar.gz'",
        kernel_path.display(),
        current_dir.display(),
        kernel_name
    );

    println!("bsdtar command: {}", bsdtar_command);

    // Execute the command
    let output = Command::new("sh")
        .arg("-c")
        .arg(bsdtar_command)
        .output()
        .await
        .context("Running bsdtar command failed")?;

    // Check the execution status and handle errors
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "bsdtar command failed with error: {}",
            stderr
        ));
    }

    //println!("Package compressed to: {}", tarball_path.display());
    Ok(())
}

pub async fn apply_patches_and_handle_conflicts(theme: &ColorfulTheme) -> Result<()> {
    let kernels_dir = Path::new("./kernels/");
    let kernel_versions = list_kernel_packages(kernels_dir).await?;

    let mut kernel_versions_items = Vec::new();
    for kernel_version in &kernel_versions {
        kernel_versions_items.push(kernel_version.as_str());
    }

    let selection = Select::with_theme(theme)
        .with_prompt("Select a kernel to patch")
        .items(&kernel_versions_items)
        .default(0)
        .interact()?;

    let selected_kernel_version = &kernel_versions[selection];
    let kernel_src_dir = kernels_dir.join(selected_kernel_version);

    let patches_dir = Path::new("./patches");
    let kernel_patch_dir = patches_dir.join(selected_kernel_version);

    let mut patch_files = Vec::new();
    for patch in WalkDir::new(&kernel_patch_dir) {
        let patch = patch?;
        if patch.path().is_file() {
            let path = patch.path();
            if let Some(extension) = path.extension() {
                if extension == "patch" {
                    patch_files.push(path.to_path_buf());
                }
            }
        }
    }

    for patch in patch_files {
        //let patch_file_path = patch.display().to_string();

        // change the patch file path to a relative path with ../../
        let relative_patch_file_path = PathBuf::from("../../")
            .join(patch.strip_prefix("./")?)
            .clone();
        let status = AsyncCommand::new("patch")
            .arg("-Np1")
            .arg("--merge")
            .arg("-i")
            .arg(&relative_patch_file_path)
            .arg("-d")
            .arg(&kernel_src_dir)
            .status()
            .await
            .context(format!("Failed to apply patch {:?}", patch.display()))?;

        if !status.success() {
            println!("Failed to apply patch. Searching for merge conflicts...");

            let conflicts = search_merge_conflicts(&kernel_src_dir).await?;
            if conflicts.is_empty() {
                println!("No merge conflicts found.");
            } else {
                handle_conflicts(conflicts.clone()).await?;
            }

            return Ok(());
        }
    }

    Ok(())
}

async fn search_merge_conflicts(
    kernel_src_dir: &Path,
) -> Result<HashMap<String, Vec<(usize, usize)>>> {
    let mut conflicts = HashMap::new();
    let rg_command = Command::new("rg")
        .arg("-n")
        .arg("--no-messages")
        .arg(r"^<<<<<<<|^=======$|^>>>>>>>")
        .arg(kernel_src_dir)
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to start ripgrep")?;

    let rg_output = rg_command
        .wait_with_output()
        .await
        .context("Failed to wait on ripgrep")?;
    if !rg_output.status.success() {
        return Err(anyhow::anyhow!("Ripgrep search failed"));
    }

    let output_str =
        String::from_utf8(rg_output.stdout).context("Failed to parse ripgrep output")?;
    let line_regex = Regex::new(r"^(.*?):(\d+):.*$").unwrap();

    let mut current_file = String::new();
    let mut current_conflict: Option<(usize, usize)> = None;

    for line in output_str.lines() {
        if let Some(caps) = line_regex.captures(line) {
            let file = caps.get(1).unwrap().as_str().to_string();
            let line_num: usize = caps.get(2).unwrap().as_str().parse().unwrap();

            if current_file != file {
                if let Some(conflict) = current_conflict.take() {
                    conflicts
                        .entry(current_file.clone())
                        .or_insert_with(Vec::new)
                        .push(conflict);
                }
                current_file = file.clone();
            }

            match line.chars().last().unwrap() {
                '<' => {
                    if let Some(conflict) = current_conflict.take() {
                        conflicts
                            .entry(current_file.clone())
                            .or_insert_with(Vec::new)
                            .push(conflict);
                    }
                    current_conflict = Some((line_num, 0)); // Start of a new conflict
                }
                '>' => {
                    if let Some(ref mut conflict) = current_conflict {
                        conflict.1 = line_num; // End of the current conflict
                    }
                }
                '=' => {} // Ignore '=======' line for now
                _ => {}
            }
        }
    }

    // Catch any trailing conflict not followed by a new file or EOF
    if let Some(conflict) = current_conflict {
        conflicts
            .entry(current_file)
            .or_insert_with(Vec::new)
            .push(conflict);
    }

    Ok(conflicts)
}

async fn handle_conflicts(conflicts: HashMap<String, Vec<(usize, usize)>>) -> Result<()> {
    let theme = ColorfulTheme::default();
    for (file_path_str, ranges) in conflicts {
        println!("Found conflict in file: {}", file_path_str);

        let file_path = PathBuf::from(&file_path_str);
        let file = File::open(&file_path).await?;
        let reader = BufReader::new(file);

        let mut lines = reader.lines();
        let mut line_number = 0;
        let mut buffer = Vec::new();

        while let Some(line) = lines.next_line().await? {
            line_number += 1;
            buffer.push((line_number, line));
        }

        println!("Displaying conflicted lines for file: {}", file_path_str);
        for (start, end) in ranges {
            // Display conflicted lines
            println!("Conflict between lines {} and {}:", start, end);

            Command::new("bat")
                .args(&[
                    "--color=always",
                    "--line-range",
                    &format!("{}:{}", start, end),
                    &file_path_str,
                ])
                .status()
                .await?;

            let selections = [
                "Accept Incoming",
                "Keep Current",
                "Accept Both",
                "Open in Editor",
            ];
            let selection = Select::with_theme(&theme)
                .with_prompt("Choose an option for resolving conflict")
                .default(0)
                .items(&selections[..])
                .interact()?;

            match selections[selection] {
                "Accept Incoming" => accept_incoming(&file_path_str, start, end).await?,
                "Keep Current" => keep_current(&file_path_str, start, end).await?,
                "Accept Both" => accept_both(&file_path_str, start, end).await?,
                "Open in Editor" => {
                    // Launch editor
                    // remove ./ before passing to nano

                    let clean_file_path = file_path_str.strip_prefix("./").unwrap();
                    if let Err(e) = AsyncCommand::new("nano")
                        .arg(format!(" {}", clean_file_path)) // Nano uses +line,column syntax
                        .status()
                        .await
                    {
                        eprintln!("Failed to open editor: {}", e);
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

async fn apply_changes_to_file<F: FnOnce(&[String]) -> Vec<String>>(
    file_path: &str,
    start: usize,
    end: usize,
    modify_lines: F,
) -> Result<()> {
    let file = fs::read_to_string(file_path).await?;
    let lines: Vec<String> = file.lines().map(String::from).collect();

    let before = lines[..start - 1].to_vec();
    let conflict = lines[start - 1..end].to_vec();
    let after = lines[end..].to_vec();

    let modified_conflict = modify_lines(&conflict);

    let modified_lines = [before, modified_conflict, after].concat();
    fs::write(file_path, modified_lines.join("\n")).await?;

    Ok(())
}

async fn accept_incoming(file_path: &str, start: usize, end: usize) -> Result<()> {
    apply_changes_to_file(file_path, start, end, |conflict_lines| {
        conflict_lines
            .iter()
            .skip_while(|line| !line.starts_with("======="))
            .skip(1) // Skip the ======= line itself
            .take_while(|line| !line.starts_with(">>>>>>>"))
            .cloned()
            .collect()
    })
    .await?;

    println!("Accepted incoming changes for {}", file_path);
    Ok(())
}

async fn keep_current(file_path: &str, start: usize, end: usize) -> Result<()> {
    apply_changes_to_file(file_path, start, end, |conflict_lines| {
        conflict_lines
            .iter()
            .take_while(|line| !line.starts_with("======="))
            .cloned()
            .collect()
    })
    .await?;

    println!("Kept current changes for {}", file_path);
    Ok(())
}

async fn accept_both(file_path: &str, start: usize, end: usize) -> Result<()> {
    apply_changes_to_file(file_path, start, end, |conflict_lines| {
        conflict_lines
            .iter()
            .filter(|line| {
                !line.starts_with("<<<<<<<")
                    && !line.starts_with("=======")
                    && !line.starts_with(">>>>>>>")
            })
            .cloned()
            .collect()
    })
    .await?;

    println!("Merged both changes for {}", file_path);
    Ok(())
}
