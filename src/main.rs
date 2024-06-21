use anyhow::{Context, Result};
use clap::Parser;
use dialoguer::{theme::ColorfulTheme, Input, Select};
use dirs_next::config_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::fs::File;
use std::io::{self, BufRead};
use std::path::Path;
use std::path::PathBuf;
use std::str;
use tokio::process::Command;
use fs_extra::dir::{copy, CopyOptions};
use tokio::process::Command as TokioCommand;
use directories::BaseDirs;

mod pkg_manager;

async fn fetch_kernel_config_options() -> Result<Vec<String>> {
    let file_path = "kernel_options.txt"; // Adjust the path to where your file is located
    let file = File::open(file_path).context("Failed to open kernel options file")?;

    let buf_reader = io::BufReader::new(file);

    // Here, we use map and filter_map to handle potential errors in line reading
    let options = buf_reader
        .lines()
        .filter_map(|line_result| line_result.ok()) // Ignore lines that fail to read
        .collect::<Vec<String>>(); // Collect into Vec<String>

    Ok(options)
}
async fn search_and_configure_option(theme: &ColorfulTheme) -> Result<()> {
    // Fetch available options (simulated here)
    let available_options = fetch_kernel_config_options().await;

    // Get user input for search
    let search_query: String = Input::with_theme(theme)
        .with_prompt("Search Kernel Options (leave empty to list all)")
        .allow_empty(true)
        .interact_text()
        .context("Failed to read input")?;

    // Filter options based on search query
    let filtered_options: Vec<String> = if search_query.is_empty() {
        available_options?
    } else {
        available_options?
            .iter()
            .filter(|option| option.to_lowercase().contains(&search_query.to_lowercase()))
            .map(|option| option.clone())
            .collect()
    };

    // If only one option matches, or if listing all without a search query, proceed directly
    if filtered_options.len() == 1 || search_query.is_empty() {
        // Present the filtered options to the user
        let selection = Select::with_theme(theme)
            .with_prompt("Select an option to configure")
            .items(&filtered_options)
            .default(0)
            .interact()
            .context("Failed to select an option")?;

        // Placeholder for configuring the selected option
        println!("Configuring: {}", filtered_options[selection]);
        // Implement the configuration change here
    }

    Ok(())
}

//use reqwest;
extern crate scraper;

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("Request failed")]
    Request(#[from] reqwest::Error),
    #[error("Failed to parse version information")]
    Parse,
}

#[derive(Parser, Debug)]
#[clap(name = "capyCachy kcli", version)]
struct CliArgs {
    #[clap(long)]
    auto_accept_defaults: bool,
    #[clap(long)]
    install: bool, // This flag will be true if -install is used
    #[clap(long, requires("install"))] // Only accept this if --install is also used
    file_path: Option<String>, // Optional path to the .tar.gz file
    #[clap(long)]
    uninstall: bool, // This flag will be true if --uninstall is used
    #[clap(long, requires("uninstall"))] // Only accept this if --install is also used
    kernel_version: Option<String>, // Optional kernel version to uninstall
    #[clap(long)]
    list: bool, // This flag will be true if --list is used
}

use std::process;

async fn execute_custom_command(file_path: Option<String>) -> Result<()> {
    // Check if executed with sudo or as root
    if !nix::unistd::Uid::effective().is_root() {
        eprintln!("This command must be executed as sudo or root.");
        process::exit(1);
    }

    // Ensure a file path is provided
    let file_path = match file_path {
        Some(fp) => fp,
        None => {
            eprintln!("No file path provided.");
            return Err(anyhow::anyhow!("No file path provided"));
        }
    };

    // Check if the file is a .tar.gz
    if !file_path.ends_with(".tar.gz") {
        eprintln!("The file must be a .tar.gz archive.");
        return Err(anyhow::anyhow!("Invalid file type"));
    }

    // Check for .srctree file inside the .tar.gz without extracting everything
    let tar_tz_command = format!("tar -tzf {} | grep -q '.srctree'", file_path);
    let tar_output = Command::new("sh")
        .arg("-c")
        .arg(&tar_tz_command)
        .output()
        .await
        .context("Failed to list contents of tar.gz file")?;

    if !tar_output.status.success() {
        eprintln!("The .tar.gz file does not contain a .srctree file.");
        return Err(anyhow::anyhow!(".srctree file not found in archive"));
    }

    // If .srctree file exists, uncompress the .tar.gz to the / directory
    let tar_xz_command = format!("tar -xzf {} -C /", file_path);
    let tar_extract_output = Command::new("sh")
        .arg("-c")
        .arg(&tar_xz_command)
        .output()
        .await
        .context("Failed to extract .tar.gz file")?;

    // mv .srctree to XDG_CONFIG_HOME/kcli/kernel_version/.srctree

    if let Some(mut config_path) = config_dir() {
        config_path.push("kcli");
        fs::create_dir_all(&config_path)?; // Ensure the directory exists

        // create dir for kernel version extracted from file_path
        let kernel_version = file_path
            .split('/')
            .last()
            .unwrap()
            .split(".tar.gz")
            .next()
            .unwrap();
        config_path.push(kernel_version);
        fs::create_dir_all(&config_path)?; // Ensure the directory exists

        // move .srctree to XDG_CONFIG_HOME/kcli/$kernel_version/.srctree
        // mv /.srctree XDG_CONFIG_HOME/kcli/$kernel_version/.srctree
        let mv_srctree_command = format!("mv /.srctree {}/.srctree", config_path.to_str().unwrap());
        let mv_srctree_output = Command::new("sh")
            .arg("-c")
            .arg(&mv_srctree_command)
            .output()
            .await
            .context("Failed to move .srctree file")?;

        if mv_srctree_output.status.success() {
            println!(".srctree file moved successfully.");
        } else {
            eprintln!("Failed to move .srctree file.");
            return Err(anyhow::anyhow!("Failed to move .srctree file"));
        }
    }

    if tar_extract_output.status.success() {
        println!("Archive extracted successfully to /.");
    } else {
        eprintln!("Failed to extract archive.");
        return Err(anyhow::anyhow!("Extraction failed"));
    }

    Ok(())
}

async fn execute_list_command() -> Result<()> {
    // list the installed kernels, by listing config_path
    let config_path: PathBuf = config_dir().unwrap().join("kcli");
    let kernel_versions = fs::read_dir(&config_path)
        .context("Failed to read kernel versions directory")?
        .filter_map(|entry| {
            entry
                .ok()
                .and_then(|e| e.file_name().into_string().ok())
                .filter(|name| name != "kernel_config.json")
        })
        .collect::<Vec<String>>();

    // remove "options" from the list
    let kernel_versions = kernel_versions
        .iter()
        .filter(|&x| x != "options")
        .collect::<Vec<&String>>();

    if kernel_versions.is_empty() {
        println!("No installed kernels found.");
    } else {
        println!("Installed kernels:");
        for kernel in kernel_versions.iter() {
            println!("- {}", kernel);
        }
    }

    Ok(())
}

async fn execute_uninstall_command(kernel_name: Option<String>) -> Result<()> {
    // list the installed kernels, by listing config_path
    println!("Uninstalling kernel {}", kernel_name.clone().unwrap());

    // get .srctree from config
    let config_path: PathBuf = config_dir().unwrap().join("kcli");
    let kernel_version_path = config_path.join(kernel_name.clone().unwrap());

    // inside the directory shall reside a .srctree file, check if exists
    let srctree_path = kernel_version_path.join(".srctree");
    if !srctree_path.exists() {
        eprintln!("No .srctree file found in the kernel version directory.");
        return Err(anyhow::anyhow!("No .srctree file found"));
    }

    // if it exists, we will rm -rf each of the files in the .srctree file
    let srctree_file = fs::read_to_string(&srctree_path).context("Failed to read .srctree file")?;
    let srctree_file = srctree_file.trim(); // Remove any leading/trailing whitespace

    // remove each file in the .srctree file
    let rm_rf_command = format!("rm -rf {}", srctree_file);
    let rm_rf_output = Command::new("sh")
        .arg("-c")
        .arg(&rm_rf_command)
        .output()
        .await
        .context("Failed to remove kernel files")?;

    if rm_rf_output.status.success() {
        println!("Kernel files removed successfully.");
    } else {
        eprintln!("Failed to remove kernel files.");
        return Err(anyhow::anyhow!("Failed to remove kernel files"));
    }

    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct KernelConfig {
    architecture: String,
    cpusched_selection: String,
    llvm_lto_selection: String,
    tick_rate: String,
    nr_cpus: String,
    hugepages: String,
    lru: String,
    tick_type: String,
    preempt_type: String,
}

impl KernelConfig {
    fn save_to_file(&self) -> std::io::Result<()> {
        if let Some(mut config_path) = config_dir() {
            config_path.push("kcli");
            fs::create_dir_all(&config_path)?; // Ensure the directory exists

            config_path.push("kernel_config.json");
            let serialized = serde_json::to_string_pretty(self)?;
            fs::write(config_path, serialized)?;
        }
        Ok(())
    }
}

impl Default for KernelConfig {
    fn default() -> Self {
        Self {
            architecture: "native".to_string(), // Default to x86_64, adjust as necessary
            cpusched_selection: "None".to_string(), // Assuming 'None' as a default value
            llvm_lto_selection: "None".to_string(), // Default to no LTO
            tick_rate: "600".to_string(),       // Common default for many Linux distros
            nr_cpus: "400".to_string(),         // Default to 4 CPUs
            hugepages: "Always".to_string(),    // Default to not using hugepages
            lru: "Standard".to_string(),        // Default LRU configuration
            tick_type: "Periodic".to_string(),  // Default tick type
            preempt_type: "Voluntary".to_string(), // Default preempt type
        }
    }
}

impl KernelConfig {
    fn load_or_default() -> Self {
        if let Some(mut path) = config_dir() {
            path.push("kcli/options/kernel_config.json");
            if let Ok(contents) = fs::read_to_string(&path) {
                if let Ok(config) = serde_json::from_str(&contents) {
                    return config;
                }
            }
        }
        Self::default()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = CliArgs::parse();
    let mut config = KernelConfig::load_or_default(); // Load the existing config or use default

    if args.list {
        execute_list_command().await?;
        return Ok(());
    }

    if args.install {
        execute_custom_command(args.file_path).await?;
        return Ok(());
    }

    if args.uninstall {
        execute_uninstall_command(args.kernel_version).await?;
        return Ok(());
    }
    let theme = ColorfulTheme::default();

    print_ascii_art().await;
    let cpu_architecture = autodetect_cpu_architecture().await?;
    println!("CPU Architecture: {}", cpu_architecture);

    let kver = fetch_latest_kernel_link().await?;
    println!("Latest Kernel Stable: {}", kver);
    println!();

    list_installed_kernels()?;
    println!();
    if !args.auto_accept_defaults {
        main_menu(&mut config, &theme).await?;
    }

    println!("Final Kernel Configuration: {:?}", config);

    Ok(())
}

fn list_installed_kernels() -> Result<()> {
    let paths = fs::read_dir("/usr/src").context("Failed to read /usr/src directory")?;

    let mut kernels = vec![];

    for path in paths {
        let path = path.context("Failed to read directory entry")?.path();
        if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
            if filename.starts_with("linux-") {
                kernels.push(filename.to_string());
            }
        }
    }

    if kernels.is_empty() {
        println!("No installed kernels found in /usr/src.");
    } else {
        println!("Installed kernels:");
        for kernel in kernels.iter() {
            println!("- {}", kernel);
        }
    }

    Ok(())
}

async fn autodetect_cpu_architecture() -> Result<String> {
    let command =
        "gcc -Q -march=native --help=target | grep -m1 march= | awk '{print toupper($2)}'";
    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
        .await // Corrected to async context
        .context("Failed to execute command")?;

    if output.status.success() {
        let arch = str::from_utf8(&output.stdout)
            .context("Failed to parse output")?
            .trim()
            .to_string();
        Ok(arch)
    } else {
        let error_message =
            str::from_utf8(&output.stderr).context("Failed to read error message")?;
        Err(anyhow::anyhow!("Command failed: {}", error_message))
    }
}

async fn fetch_latest_kernel_link() -> Result<String> {
    let command = r#"
    curl -s https://www.kernel.org | 
    grep -A 1 'id="latest_link"' | 
    awk 'NR==2' | 
    grep -oP 'href="\K[^"]+' | 
    grep -oP 'linux-\K[^"]+' |
    xargs basename -s .tar.xz
"#;

    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
        .await // Corrected to async context
        .context("Failed to execute command")?;

    if output.status.success() {
        let link = str::from_utf8(&output.stdout)
            .context("Failed to parse output")?
            .trim()
            .to_string();
        Ok(link)
    } else {
        let error_message =
            str::from_utf8(&output.stderr).context("Failed to read error message")?;
        Err(anyhow::anyhow!("Command failed: {}", error_message))
    }
}

async fn print_ascii_art() {
    println!(
        r#"
   /''''''''''''/
  /''''''''''''/
 /''''''/
/''''''/
\......\
 \......\
  \.............../
   \............./

   capyCachy kernel manager"#
    );
    println!();
}
async fn configure_kernel_options(
    config: &mut KernelConfig,
    theme: &ColorfulTheme,
    packages_dir: &Path,
) -> Result<()> {
    // First, list available Linux versions

    loop {
        let selections = vec![
            "CPU Scheduler",
            "LLVM LTO",
            "Tick Rate",
            "Hugepages",
            "LRU",
            "Tick Type",
            "Preempt Type",
            "System Optimizations",
            "<-",
        ];

        let selection = Select::with_theme(theme)
            .with_prompt("Configure Kernel Options")
            .items(&selections)
            .default(0)
            .interact()?;

        match selections[selection] {
            "CPU Scheduler" => configure_cpusched(config, theme)?,
            "LLVM LTO" => configure_llvm_lto(config, theme)?,
            "Tick Rate" => configure_tick_rate(config, theme)?,
            "Hugepages" => configure_hugepages(config, theme)?,
            "LRU" => configure_lru(config, theme)?,
            "Tick Type" => configure_tick_type(config, theme)?,
            "Preempt Type" => configure_preempt_type(config, theme)?,
            "System Optimizations" => configure_system_optimizations(config, theme)?,
            "<-" => {
                println!("Saving and returning to main menu...");
                config.save_to_file()?; // Saves the config
                break; // Exits the loop
            }
            _ => {}
        }
    }

    Ok(())
}

async fn main_menu(config: &mut KernelConfig, theme: &ColorfulTheme) -> Result<()> {
    loop {
        let selections = vec![
            "Download Kernel Source",
            "Configure Kernel Options",
            "Apply Kernel Configuration",
            "Patch Kernel", // New option for patching kernel
            "Build Kernel",
            "Package Kernel", // New option for installing kernel
            //"Uninstall Kernel", // New option for uninstalling kernel
            "Advanced Search/Configure",
            "Exit",
        ];

        let selection = Select::with_theme(theme)
            .with_prompt("Kernel Configuration Menu")
            .items(&selections)
            .default(0)
            .interact()?;

        let mut config_path = config_dir().unwrap();
        config_path.push("kcli");
        fs::create_dir_all(&config_path)?;
        let packages_dir = config_path.join("ksrc");

        match selections[selection] {
            "Download Kernel Source" => configure_download_kernel(config, theme).await?,
            "Configure Kernel Options" => {
                configure_kernel_options(config, theme, &packages_dir).await?
            }
            "Apply Kernel Configuration" => {
                apply_kernel_configuration(config, theme, &packages_dir).await?
            }
            "Build Kernel" => build_kernel_menu(config, theme, &packages_dir).await?,
            "Patch Kernel" => patch_kernel_process(theme, &packages_dir).await?,
            "Package Kernel" => pkg_manager::menu_install_kernel(theme).await?, // Implementation needed
            //"Uninstall Kernel" => pkg_manager::menu_uninstall_kernel(theme).await?, // Implementation needed
            "Advanced Search/Configure" => search_and_configure_option(theme).await?,
            "Exit" => break,
            _ => {}
        }
    }

    Ok(())
}

async fn patch_kernel_process(theme: &ColorfulTheme, packages_dir: &Path) -> Result<()> {
    let packages = pkg_manager::list_kernel_packages(packages_dir)
        .await
        .context("Failed to list kernel packages")?;

    // If no packages are found, return an error or a message
    if packages.is_empty() {
        return Err(anyhow::anyhow!("No kernel packages found."));
    }

    // Add <- Go Back to Main Menu option
    let mut packages = packages;
    packages.push("<- Back to Main Menu".to_string());

    // Prompt the user to select a Linux version
    let selected_package_index = Select::with_theme(theme)
        .with_prompt("Select a Linux version to configure")
        .items(&packages)
        .default(0)
        .interact()?;

    // If <- Back to Main Menu is selected, return to main menu
    if selected_package_index == packages.len() - 1 {
        return Ok(());
    }

    // Get the selected package name
    let selected_package = &packages[selected_package_index];
    println!("Selected package for configuration: {}", selected_package);

    // Enter the kernel directory
    let kernel_dir = Path::new(packages_dir).join(selected_package);

    // Clone or use existing patches directory
    let patches_dir = clone_patches_repo().await?;
    let selected_patch = navigate_and_select_patch(patches_dir).await?;

    if let Some(patch) = selected_patch {
        apply_patch(patch, &kernel_dir).await?;
    }

    Ok(())
}
async fn clone_patches_repo() -> Result<PathBuf, anyhow::Error> {
    let repo_url = "https://github.com/CachyOS/kernel-patches";

    // Get the configuration directory
    let base_dirs = BaseDirs::new().context("Failed to get base directories")?;
    let mut config_path = base_dirs.config_dir().to_path_buf();
    config_path.push("kcli");
    config_path.push("kernel-patches");

    // Check if the target directory already exists
    if config_path.exists() {
        println!("Kernel patches directory already exists at '{}'.", config_path.to_string_lossy());
        return Ok(config_path);
    }

    // Temporary directory for cloning to avoid name conflict
    let tmp_dir = "/tmp/kernel_patches_tmp";

    // Clone the repository into the temporary directory
    if Path::new(tmp_dir).exists() {
        fs::remove_dir_all(tmp_dir).context("Failed to remove existing temporary directory")?;
    }

    Command::new("git")
        .args(["clone", repo_url, tmp_dir])
        .output()
        .await
        .context("Failed to clone kernel patches repository")?;

    // Copy the directory instead of renaming
    let tmp_path = Path::new(tmp_dir);
    if tmp_path.exists() {
        let mut options = CopyOptions::new();
        options.copy_inside = true;
        copy(tmp_path, &config_path, &options)
            .context("Failed to copy kernel patches to config directory")?;
        // Clean up temporary directory
        fs::remove_dir_all(tmp_path).context("Failed to remove temporary directory")?;
    } else {
        return Err(anyhow::anyhow!(
            "Temporary directory not found after cloning."
        ));
    }

    Ok(config_path)
}


async fn navigate_and_select_patch(patch_dir: PathBuf) -> Result<Option<PathBuf>> {
    let mut current_dir = patch_dir;
    let mut history = Vec::new(); // Stack to track the path history

    loop {
        // Spawn a blocking task to read and process the directory
        let current_dir_clone = current_dir.clone(); // Clone for moving into the closure
        let mut entries_vec = tokio::task::spawn_blocking(move || {
            let mut entries = Vec::new();
            if let Ok(entries_iter) = fs::read_dir(&current_dir_clone) {
                for entry in entries_iter.flatten() {
                    let path = entry.path();
                    // Filter out the .git directory
                    if path.file_name().and_then(std::ffi::OsStr::to_str) == Some(".git") {
                        continue; // Skip the .git directory
                    }
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        entries.push((name.to_owned(), path));
                    }
                }
            }
            entries
        })
        .await
        .context("Failed to read directory entries")?; // Await the result of the blocking operation

        // Sort entries: directories first, then files
        entries_vec.sort_by_key(|(_, path)| (!path.is_dir(), path.clone()));

        // Get names for display in the menu
        let mut options = entries_vec
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();

        // Add options for navigation control
        if !history.is_empty() {
            options.insert(0, "<- Go Back".to_string()); // Option to go back in history
        } else {
            options.insert(0, "<- Back to Main Menu".to_string()); // Option to return to main menu from top directory
        }

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Select a folder or patch file")
            .items(&options)
            .default(0)
            .interact()?;

        // Handling the "Go back" option
        if selection == 0 {
            if !history.is_empty() {
                current_dir = history.pop().unwrap(); // Navigate back in the directory stack
                continue;
            } else {
                return Ok(None); // No selection made, return to main menu
            }
        }

        // Properly adjust the index for selecting entries
        let selected_path = &entries_vec[selection - 1].1; // Adjust index by one to account for the go back option

        // Check if it's a directory or a .patch file
        if selected_path.is_dir() {
            history.push(current_dir.clone()); // Push current directory to history
            current_dir = selected_path.clone();
        } else if selected_path.extension().and_then(std::ffi::OsStr::to_str) == Some("patch") {
            return Ok(Some(selected_path.clone()));
        }
    }
}

async fn apply_patch(patch_file: PathBuf, kernel_dir: &Path) -> Result<(), anyhow::Error> {
    let patch_file_str = patch_file.to_string_lossy().to_string();

    // Command to apply the patch
    let output = Command::new("sh")
        .current_dir(kernel_dir) // Sets the working directory to kernel_dir
        .arg("-c")
        .arg(format!("patch -Np1 --merge < {}", patch_file_str))
        .output()
        .await
        .context("Failed to apply patch")?;

    // Check if the command was successful
    if !output.status.success() {
        let error_message = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to apply patch: {}", error_message);
    }

    println!(
        "Patch applied successfully: {}",
        patch_file.to_string_lossy()
    );

    Ok(())
}

async fn modify_kernel_config(kernel_dir: &str, config_commands: Vec<String>) -> Result<()> {
    // Ensure the kernel directory exists and has a scripts/config file
    if !std::path::Path::new(&format!("{}/scripts/config", kernel_dir)).exists() {
        return Err(anyhow::anyhow!(
            "Kernel directory does not contain scripts/config"
        ));
    }

    for command in config_commands {
        let args: Vec<&str> = command.split_whitespace().collect();
        let status = Command::new("sh")
            .arg("-c")
            .arg(format!("{}/scripts/config {}", kernel_dir, command))
            .current_dir(kernel_dir) // Ensure we're in the right directory
            .status()
            .await
            .context("Failed to execute kernel config command")?;

        if !status.success() {
            eprintln!("Failed to execute command: {}", command);
            return Err(anyhow::anyhow!("Kernel configuration command failed"));
        }
    }

    println!("Kernel configuration updated successfully.");

    Ok(())
}

async fn build_kernel_menu(
    config: &mut KernelConfig,
    theme: &ColorfulTheme,
    packages_dir: &Path,
) -> Result<()> {
    // First, list available Linux versions
    let packages = pkg_manager::list_kernel_packages(packages_dir)
        .await
        .context("Failed to list kernel packages")?;

    // If no packages are found, return an error or a message
    if packages.is_empty() {
        return Err(anyhow::anyhow!("No kernel packages found."));
    }

    // add <- Go Back to Main Menu option
    let mut packages = packages;
    packages.push("<- Back to Main Menu".to_string());

    // Prompt the user to select a Linux version
    let selected_package_index = Select::with_theme(theme)
        .with_prompt("Select a Linux version to configure")
        .items(&packages)
        .default(0)
        .interact()?;

    // if <- Back to Main Menu is selected, return to main menu
    if selected_package_index == packages.len() - 1 {
        return Ok(());
    }

    // Get the selected package name
    let selected_package = &packages[selected_package_index];
    println!("Selected package for configuration: {}", selected_package);

    // now we should enter the kernel directory
    let kernel_dir = Path::new(packages_dir).join(selected_package);

    loop {
        let selections = vec![
            "Compile",
            "Install Modules",
            "Install Headers",
            "<- Back to Main Menu",
        ];

        let selection = Select::with_theme(theme)
            .with_prompt("Build Kernel")
            .items(&selections)
            .default(0)
            .interact()?;

        match selections.get(selection) {
            Some(&"Compile") => {
                run_make_command(
                    "LOCALVERSION=\"-capy\" KCFLAGS=\"-mpopcnt -fivopts -fmodulo-sched\"",
                    &kernel_dir,
                )
                .await?;
            }
            Some(&"Install Modules") => {
                run_make_command("modules_install", &kernel_dir).await?;
            }
            Some(&"Install Headers") => {
                run_make_command("headers_install", &kernel_dir).await?;
            }
            Some(&"<- Back to Main Menu") => return Ok(()),
            _ => return Err(anyhow::anyhow!("Invalid selection")),
        }
    }
}

async fn run_make_command(args: &str, kernel_dir: &Path) -> Result<()> {
    let command = format!("make {}", args);
    use shell_words::split; // Add shell_words to your Cargo.toml

    // print kernel_dir
    
    let config_path = kernel_dir.join(".config");
    // print config path

    println!("Running make command: {}", command);
    // print config_path
    println!("config path: {}", config_path.display());

    //let version_path = kernel_dir.join("version"); // Path for the version file

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

    // run make kernelversion > version
    let status = Command::new("make")
        .args(split(&command).context("Failed to parse command arguments")?)
        .current_dir(kernel_dir) // Use the provided kernel directory
        .status()
        .await
        .context("Failed to execute make command")?;

    if status.success() {
        println!("Command executed successfully.");
    } else {
        eprintln!("Command execution failed.");
        return Err(anyhow::anyhow!("Make command failed"));
    }


    let args_vec = split(&command).context("Failed to parse command arguments")?;

    let (make, args) = args_vec
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("No command found"))?;

    let status = Command::new(make)
        .args(args)
        .current_dir(kernel_dir) // Use the provided kernel directory
        .status()
        .await
        .context("Failed to execute make command")?;

    if status.success() {
        println!("Command executed successfully.");
    } else {
        eprintln!("Command execution failed.");
        return Err(anyhow::anyhow!("Make command failed"));
    }

    Ok(())
}
async fn configure_download_kernel(config: &mut KernelConfig, theme: &ColorfulTheme) -> Result<()> {
    let selections = vec!["Stable Kernel", "RC Kernel", "<-"];
    let selection = Select::with_theme(theme)
        .with_prompt("Select Kernel Version to Download")
        .items(&selections)
        .default(0)
        .interact()?;

    let (url, dir_name) = match selections[selection] {
        "Stable Kernel" => (
            "https://git.kernel.org/pub/scm/linux/kernel/git/stable/linux.git",
            "linux-stable",
        ),
        "RC Kernel" => ("https://github.com/torvalds/linux.git", "linux-rc"),
        "<-" => return Ok(()),
        _ => return Err(anyhow::anyhow!("Invalid selection")),
    };

    // Temporary directory for cloning to avoid name conflict
    let tmp_dir = "/tmp/ksrc_tmp";
    let mut config_path = config_dir().unwrap();
    config_path.push("kcli");
    config_path.push("ksrc");
    fs::create_dir_all(&config_path)?;

    let target_dir = config_path.join(dir_name);

    fs::create_dir_all(&target_dir.parent().unwrap())
        .context("Failed to create 'ksrc' directory")?;

    // Execute 'git clone' within the temporary directory
    let output = TokioCommand::new("git")
        .args(["clone", "--depth", "1", url, tmp_dir])
        .output()
        .await
        .context("Failed to execute git clone command")?;

    if !output.status.success() {
        let error_message = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("Git clone failed: {}", error_message));
    }

    // Copy the directory instead of renaming
    let tmp_path = Path::new(tmp_dir);
    if tmp_path.exists() {
        let mut options = CopyOptions::new();
        options.copy_inside = true;
        copy(tmp_path, &target_dir, &options)
            .context(format!("Failed to copy directory to '{}'", dir_name))?;
        println!(
            "Kernel source downloaded and moved to '{}'.",
            target_dir.to_string_lossy()
        );
        // Clean up temporary directory
        fs::remove_dir_all(tmp_path)
            .context("Failed to remove temporary directory")?;
    } else {
        return Err(anyhow::anyhow!(
            "Temporary directory not found after cloning."
        ));
    }

    Ok(())
}

fn configure_cpusched(config: &mut KernelConfig, theme: &ColorfulTheme) -> Result<()> {
    let selections = vec!["CachyOS", "PDS", "None"];
    let selection = Select::with_theme(theme)
        .with_prompt("CPU Scheduler Configuration")
        .items(&selections)
        .default(0)
        .interact()?;
    config.cpusched_selection = selections[selection].to_string();
    Ok(())
}

fn configure_llvm_lto(config: &mut KernelConfig, theme: &ColorfulTheme) -> Result<()> {
    let selections = vec!["Thin", "Full", "None"];
    let selection = Select::with_theme(theme)
        .with_prompt("LLVM LTO Configuration")
        .items(&selections)
        .default(0)
        .interact()?;
    config.llvm_lto_selection = selections[selection].to_string();
    Ok(())
}

fn configure_tick_rate(config: &mut KernelConfig, theme: &ColorfulTheme) -> Result<()> {
    let selections = vec!["100", "250", "500", "600", "1000"];
    let selection = Select::with_theme(theme)
        .with_prompt("Tick Rate Configuration")
        .items(&selections)
        .default(0)
        .interact()?;
    config.tick_rate = selections[selection].to_string();
    Ok(())
}

fn configure_nr_cpus(config: &mut KernelConfig, theme: &ColorfulTheme) -> Result<()> {
    let selections = vec!["1", "2", "4", "8", "16", "32", "64", "128", "256", "320"];
    let selection = Select::with_theme(theme)
        .with_prompt("NR_CPUS Configuration")
        .items(&selections)
        .default(0)
        .interact()?;
    config.nr_cpus = selections[selection].to_string();
    Ok(())
}

fn configure_hugepages(config: &mut KernelConfig, theme: &ColorfulTheme) -> Result<()> {
    let selections = vec!["Always", "Madvise", "No"];
    let selection = Select::with_theme(theme)
        .with_prompt("Hugepages Configuration")
        .items(&selections)
        .default(0)
        .interact()?;
    config.hugepages = selections[selection].to_string();
    Ok(())
}

fn configure_lru(config: &mut KernelConfig, theme: &ColorfulTheme) -> Result<()> {
    let selections = vec!["Standard", "Stats", "None"];
    let selection = Select::with_theme(theme)
        .with_prompt("LRU Configuration")
        .items(&selections)
        .default(0)
        .interact()?;
    config.lru = selections[selection].to_string();
    Ok(())
}

fn configure_tick_type(config: &mut KernelConfig, theme: &ColorfulTheme) -> Result<()> {
    let selections = vec!["Periodic", "NoHz_Full", "NoHz_Idle"];
    let selection = Select::with_theme(theme)
        .with_prompt("Tick Type Configuration")
        .items(&selections)
        .default(0)
        .interact()?;
    config.tick_type = selections[selection].to_string();
    Ok(())
}

fn configure_preempt_type(config: &mut KernelConfig, theme: &ColorfulTheme) -> Result<()> {
    let selections = vec!["Voluntary", "Preempt", "None"];
    let selection = Select::with_theme(theme)
        .with_prompt("Preempt Type Configuration")
        .items(&selections)
        .default(0)
        .interact()?;
    config.preempt_type = selections[selection].to_string();
    Ok(())
}

fn configure_system_optimizations(config: &mut KernelConfig, theme: &ColorfulTheme) -> Result<()> {
    // Placeholder: Implement system optimizations configuration
    // This function can use a combination of `Select` and `Confirm` for different types of options
    println!("Configuring System Optimizations (Placeholder)");
    Ok(())
}

async fn apply_kernel_configuration(
    config: &KernelConfig,
    theme: &ColorfulTheme,
    packages_dir: &Path,
) -> Result<()> {
    // First, list available Linux versions
    let packages = pkg_manager::list_kernel_packages(packages_dir)
        .await
        .context("Failed to list kernel packages")?;

    // If no packages are found, return an error or a message
    if packages.is_empty() {
        return Err(anyhow::anyhow!("No kernel packages found."));
    }

    // add <- Go Back to Main Menu option
    let mut packages = packages;
    packages.push("<- Back to Main Menu".to_string());

    // Prompt the user to select a Linux version
    let selected_package_index = Select::with_theme(theme)
        .with_prompt("Select a Linux version to configure")
        .items(&packages)
        .default(0)
        .interact()?;

    // if <- Back to Main Menu is selected, return to main menu
    if selected_package_index == packages.len() - 1 {
        return Ok(());
    }

    // Get the selected package name
    let selected_package = &packages[selected_package_index];
    println!("Selected package for configuration: {}", selected_package);

    // now we should enter the kernel directory
    let kernel_src_dir = Path::new(packages_dir).join(selected_package);

    let arch_config_cmd = format!("CONFIG_{}", config.architecture);
    tokio::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "{}/scripts/config --disable CONFIG_GENERIC_CPU --enable {}",
            kernel_src_dir.display(),
            arch_config_cmd
        ))
        .output()
        .await
        .context("Failed to set architecture")?;

    // CPU Scheduler Configuration
    match config.cpusched_selection.as_str() {
        "None" => {
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg("scripts/config -d SCHED_BORE -d SCHED_CLASS_EXT -d SCHED_PDS")
                .output()
                .await
                .context("Failed to disable CPU scheduler config")?;
        }
        _ => {}
    }

    // Hugepages
    match config.hugepages.as_str() {
        "Always" => {
            tokio::process::Command::new("sh")
            .arg("-c")
            .arg("scripts/config -e HUGETLBFS -e HUGETLB_PAGE -e HUGETLB -e HUGETLB_PAGE_SIZE_VARIABLE")
            .current_dir(kernel_src_dir.clone())  // Set the current directory here
            .output()
            .await
            .context("Failed to enable hugepages")?;
        }
        "Madvise" => {
            tokio::process::Command::new("sh")
            .arg("-c")
            .arg("scripts/config -e HUGETLBFS -e HUGETLB_PAGE -e HUGETLB -d HUGETLB_PAGE_SIZE_VARIABLE")
            .current_dir(kernel_src_dir.clone())  // Set the current directory here
            .output()
            .await
            .context("Failed to enable hugepages")?;
        }
        "No" => {
            tokio::process::Command::new("sh")
            .arg("-c")
            .arg("scripts/config -d HUGETLBFS -d HUGETLB_PAGE -d HUGETLB -d HUGETLB_PAGE_SIZE_VARIABLE")
            .current_dir(kernel_src_dir.clone())  // Set the current directory here
            .output()
            .await
            .context("Failed to disable hugepages")?;
        }
        _ => {}
    }

    // LRU
    match config.lru.as_str() {
        "Standard" => {
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg("scripts/config -e LRU_LIST -d LRU_STATS -d LRU")
                .current_dir(kernel_src_dir.clone()) // Set the current directory here
                .output()
                .await
                .context("Failed to configure standard LRU")?;
        }
        "Stats" => {
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg("scripts/config -d LRU_LIST -e LRU_STATS -d LRU")
                .current_dir(kernel_src_dir.clone()) // Set the current directory here
                .output()
                .await
                .context("Failed to configure LRU with stats")?;
        }
        "None" => {
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg("scripts/config -d LRU_LIST -d LRU_STATS -e LRU")
                .current_dir(kernel_src_dir.clone()) // Set the current directory here
                .output()
                .await
                .context("Failed to disable LRU")?;
        }
        _ => {}
    }

    // Preempt Type Configuration
    match config.preempt_type.as_str() {
        "full" => {
            tokio::process::Command::new("sh")
            .arg("-c")
            .arg("scripts/config -e PREEMPT_BUILD -d PREEMPT_NONE -d PREEMPT_VOLUNTARY -e PREEMPT -e PREEMPT_COUNT -e PREEMPTION -e PREEMPT_DYNAMIC")
            .current_dir(kernel_src_dir.clone()) // Set the current directory here
            .output()
            .await
            .context("Failed to configure full preemption")?;
        }
        "voluntary" => {
            tokio::process::Command::new("sh")
            .arg("-c")
            .arg("scripts/config -e PREEMPT_BUILD -d PREEMPT_NONE -e PREEMPT_VOLUNTARY -d PREEMPT -e PREEMPT_COUNT -e PREEMPTION -d PREEMPT_DYNAMIC")
            .current_dir(kernel_src_dir.clone()) // Set the current directory here
            .output()
            .await
            .context("Failed to configure voluntary preemption")?;
        }
        "none" => {
            tokio::process::Command::new("sh")
            .arg("-c")
            .arg("scripts/config -e PREEMPT_NONE_BUILD -e PREEMPT_NONE -d PREEMPT_VOLUNTARY -d PREEMPT -d PREEMPTION -d PREEMPT_DYNAMIC")
            .current_dir(kernel_src_dir.clone()) // Set the current directory here
            .output()
            .await
            .context("Failed to disable preemption")?;
        }
        _ => {}
    }

    // Tick Rate Configuration

    println!("Configuring tick rate to {}", config.tick_rate.as_str());
    match config.tick_rate.as_str() {
        "100" | "250" | "500" | "600" | "1000" => {
            let result = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "scripts/config -d HZ_300 -e HZ_{} --set-val HZ {}",
                    config.tick_rate, config.tick_rate
                ))
                .current_dir(kernel_src_dir.clone()) // Correct placement of .current_dir()
                .output()
                .await
                .context(format!(
                    "Failed to configure tick rate to {}",
                    config.tick_rate
                ));

            match result {
                Ok(output) => {
                    // Handle successful output
                    println!("Command executed successfully.");
                }
                Err(e) => {
                    // Handle errors
                    eprintln!("Failed to execute command: {}", e);
                }
            }
        }
        _ => {
            println!("Tick rate not supported");
        }
    }

    // Tick Type Configuration
    match config.tick_type.as_str() {
        "Periodic" => {
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg("scripts/config -e TICK_PERIODIC -d TICK_ONESHOT -d NO_HZ_IDLE -d NO_HZ_FULL")
                .current_dir(kernel_src_dir.clone()) // Set the current directory here
                .output()
                .await
                .context("Failed to configure tick type to Periodic")?;
        }
        "NoHz_Full" => {
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg("scripts/config -d TICK_PERIODIC -d TICK_ONESHOT -d NO_HZ_IDLE -e NO_HZ_FULL")
                .current_dir(kernel_src_dir.clone()) // Set the current directory here
                .output()
                .await
                .context("Failed to configure tick type to NoHz_Full")?;
        }
        "NoHz_Idle" => {
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg("scripts/config -d TICK_PERIODIC -e TICK_ONESHOT -e NO_HZ_IDLE -d NO_HZ_FULL")
                .current_dir(kernel_src_dir.clone()) // Set the current directory here
                .output()
                .await
                .context("Failed to configure tick type to NoHz_Idle")?;
        }
        _ => {}
    }

    Ok(())
}
