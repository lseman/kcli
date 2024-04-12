use anyhow::{Context, Result};
use clap::Parser;
use dialoguer::Confirm;
use dialoguer::{theme::ColorfulTheme, Input, Select};
use serde::{Deserialize, Serialize}; // Add serde and serde_json to Cargo.toml
use std::fs;
use std::fs::File;
use std::io::{self, BufRead};
use std::path::Path;
use std::path::PathBuf;
use std::str;
use tokio::process::Command;

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
#[clap(name = "Kernel Configuration", version)]
struct CliArgs {
    #[clap(long)]
    auto_accept_defaults: bool,
    #[clap(long)]
    install: bool, // This flag will be true if -install is used
    #[clap(long, requires("install"))] // Only accept this if --install is also used
    file_path: Option<String>, // Optional path to the .tar.gz file
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

    if tar_extract_output.status.success() {
        println!("Archive extracted successfully to /.");
    } else {
        eprintln!("Failed to extract archive.");
        return Err(anyhow::anyhow!("Extraction failed"));
    }

    Ok(())
}

#[derive(Debug)]
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

#[tokio::main]
async fn main() -> Result<()> {
    let args = CliArgs::parse();

    if args.install {
        execute_custom_command(args.file_path).await?;
        return Ok(());
    }
    let theme = ColorfulTheme::default();
    let mut config = KernelConfig::default();

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
                                 .;o,
        __."iIoi,._              ;pI __-"-xx.,_
      `.3"P3PPPoie-,.            .d' `;.     `p;
     `O"dP"````""`PdEe._       .;'   .     `  `|   NACK
    "$#"'            ``"P4rdddsP'  .F.    ` `` ;  /
   i/"""     *"Sp.               .dPff.  _.,;Gw'
   ;l"'     "  `dp..            "sWf;fe|'
  `l;          .rPi .    . "" "dW;;doe;
   $          .;PE`'       " "sW;.d.d;
   $$        .$"`     `"saed;lW;.d.d.i
   .$M       ;              ``  ' ld;.p.
__ _`$o,.-__  "ei-Mu~,.__ ___ `_-dee3'o-ii~m. ____"#
    );
    println!();
}
async fn configure_kernel_options(
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

    // Prompt the user to select a Linux version
    let selected_package_index = Select::with_theme(theme)
        .with_prompt("Select a Linux version to configure")
        .items(&packages)
        .default(0)
        .interact()?;

    // Get the selected package name
    let selected_package = &packages[selected_package_index];
    println!("Selected package for configuration: {}", selected_package);

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
            "Back to Main Menu",
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
            "Back to Main Menu" => break,
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
            "Build Kernel",
            "Patch Kernel",     // New option for patching kernel
            "Install Kernel",   // New option for installing kernel
            "Uninstall Kernel", // New option for uninstalling kernel
            "Advanced Search/Configure",
            "Exit",
        ];

        let selection = Select::with_theme(theme)
            .with_prompt("Kernel Configuration Menu")
            .items(&selections)
            .default(0)
            .interact()?;

        let packages_dir = PathBuf::from("./ksrc");

        match selections[selection] {
            "Download Kernel Source" => configure_download_kernel(config, theme).await?,
            "Configure Kernel Options" => {
                configure_kernel_options(config, theme, &packages_dir).await?
            }
            "Build Kernel" => build_kernel_menu(config, theme).await?,
            "Patch Kernel" => pkg_manager::apply_patches_and_handle_conflicts(theme).await?,
            "Install Kernel" => pkg_manager::menu_install_kernel(theme).await?, // Implementation needed
            "Uninstall Kernel" => pkg_manager::menu_uninstall_kernel(theme).await?, // Implementation needed
            "Advanced Search/Configure" => search_and_configure_option(theme).await?,
            "Exit" => break,
            _ => {}
        }
    }

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

async fn build_kernel_menu(config: &mut KernelConfig, theme: &ColorfulTheme) -> Result<()> {
    loop {
        let selections = vec!["Compile", "Install Modules", "Install Headers", "Go Back"];

        let selection = Select::with_theme(theme)
            .with_prompt("Build Kernel")
            .items(&selections)
            .default(0)
            .interact()?;

        match selections.get(selection) {
            Some(&"Compile") => {
                run_make_command("LOCALVERSION=\"\" KCFLAGS=\"-mpopcnt -fivopts -fmodulo-sched\"")
                    .await?;
            }
            Some(&"Install Modules") => {
                run_make_command("modules_install").await?;
            }
            Some(&"Install Headers") => {
                run_make_command("headers_install").await?;
            }
            Some(&"Go Back") => return Ok(()),
            _ => return Err(anyhow::anyhow!("Invalid selection")),
        }
    }
}

async fn run_make_command(args: &str) -> Result<()> {
    // Split the args into a Vec by whitespace, respecting quoted substrings
    use shell_words::split; // Add shell_words to your Cargo.toml

    let command = format!("make {}", args);
    let args_vec = split(&command).context("Failed to parse command arguments")?;

    // Use the first element as the command and the rest as args
    let (make, args) = args_vec
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("No command found"))?;

    let status = Command::new(make)
        .args(args)
        .current_dir("./linux") // Set the working directory to the linux subfolder
        .status()
        .await // Wait for the future to complete
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
    let selections = vec!["Stable Kernel", "RC Kernel", "Go Back to Main Menu"];
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
        "Go Back to Main Menu" => return Ok(()),
        _ => return Err(anyhow::anyhow!("Invalid selection")),
    };

    // Temporary directory for cloning to avoid name conflict
    let tmp_dir = "ksrc_tmp";
    let target_dir = Path::new("ksrc").join(dir_name);

    fs::create_dir_all(&target_dir.parent().unwrap())
        .context("Failed to create 'ksrc' directory")?;

    // Execute 'git clone' within the temporary directory
    let output = Command::new("git")
        .args(["clone", "--depth", "1", url, tmp_dir])
        .output()
        .await
        .context("Failed to execute git clone command")?;

    if !output.status.success() {
        let error_message = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("Git clone failed: {}", error_message));
    }

    // Rename and move the directory
    let tmp_path = Path::new(tmp_dir);
    if tmp_path.exists() {
        fs::rename(tmp_path, &target_dir)
            .context(format!("Failed to rename directory to '{}'", dir_name))?;
        println!(
            "Kernel source downloaded and moved to '{}'.",
            target_dir.to_string_lossy()
        );
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
async fn apply_kernel_configuration(config: &KernelConfig, kernel_src_dir: &str) -> Result<()> {
    // Set architecture (example setting, adjust as necessary)
    let arch_config_cmd = format!("CONFIG_{}", config.architecture);
    tokio::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "{}/scripts/config --disable CONFIG_GENERIC_CPU --enable {}",
            kernel_src_dir, arch_config_cmd
        ))
        .output()
        .await
        .context("Failed to set architecture")?;

    // CPU Scheduler Configuration
    match config.cpusched_selection.as_str() {
        "None" => {
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "{}/scripts/config -d SCHED_BORE -d SCHED_CLASS_EXT -d SCHED_PDS",
                    kernel_src_dir
                ))
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
                .arg(format!("{}/scripts/config -e HUGETLBFS -e HUGETLB_PAGE -e HUGETLB -e HUGETLB_PAGE_SIZE_VARIABLE", kernel_src_dir))
                .output()
                .await
                .context("Failed to enable hugepages")?;
        }
        "Madvise" => {
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(format!("{}/scripts/config -e HUGETLBFS -e HUGETLB_PAGE -e HUGETLB -d HUGETLB_PAGE_SIZE_VARIABLE", kernel_src_dir))
                .output()
                .await
                .context("Failed to enable hugepages")?;
        }
        "No" => {
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(format!("{}/scripts/config -d HUGETLBFS -d HUGETLB_PAGE -d HUGETLB -d HUGETLB_PAGE_SIZE_VARIABLE", kernel_src_dir))
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
                .arg(format!(
                    "{}/scripts/config -e LRU_LIST -d LRU_STATS -d LRU",
                    kernel_src_dir
                ))
                .output()
                .await
                .context("Failed to configure standard LRU")?;
        }
        "Stats" => {
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "{}/scripts/config -d LRU_LIST -e LRU_STATS -d LRU",
                    kernel_src_dir
                ))
                .output()
                .await
                .context("Failed to configure LRU with stats")?;
        }
        "None" => {
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "{}/scripts/config -d LRU_LIST -d LRU_STATS -e LRU",
                    kernel_src_dir
                ))
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
                .arg(format!("{}/scripts/config -e PREEMPT_BUILD -d PREEMPT_NONE -d PREEMPT_VOLUNTARY -e PREEMPT -e PREEMPT_COUNT -e PREEMPTION -e PREEMPT_DYNAMIC", kernel_src_dir))
                .output()
                .await
                .context("Failed to configure full preemption")?;
        }
        "voluntary" => {
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(format!("{}/scripts/config -e PREEMPT_BUILD -d PREEMPT_NONE -e PREEMPT_VOLUNTARY -d PREEMPT -e PREEMPT_COUNT -e PREEMPTION -d PREEMPT_DYNAMIC", kernel_src_dir))
                .output()
                .await
                .context("Failed to configure voluntary preemption")?;
        }
        "none" => {
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(format!("{}/scripts/config -e PREEMPT_NONE_BUILD -e PREEMPT_NONE -d PREEMPT_VOLUNTARY -d PREEMPT -d PREEMPTION -d PREEMPT_DYNAMIC", kernel_src_dir))
                .output()
                .await
                .context("Failed to disable preemption")?;
        }

        _ => {}
    }

    // Tick Rate Configuration
    match config.tick_rate.as_str() {
        "100" | "250" | "500" | "600" | "1000" => {
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "{}/scripts/config -d HZ_300 -e HZ_{} --set-val HZ {}",
                    kernel_src_dir, config.tick_rate, config.tick_rate
                ))
                .output()
                .await
                .context(format!(
                    "Failed to configure tick rate to {}",
                    config.tick_rate
                ))?;
        }
        _ => {}
    }

    // Tick Type Configuration
    match config.tick_type.as_str() {
        "Periodic" => {
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(format!("{}/scripts/config -e TICK_ONESHOT -d TICK_ONESHOT -d TICK_ONESHOT -d TICK_ONESHOT", kernel_src_dir))
                .output()
                .await
                .context("Failed to configure tick type to periodic")?;
        }
        "NoHz_Full" => {
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(format!("{}/scripts/config -d TICK_ONESHOT -e TICK_ONESHOT -d TICK_ONESHOT -d TICK_ONESHOT", kernel_src_dir))
                .output()
                .await
                .context("Failed to configure tick type to NoHz_Full")?;
        }
        "NoHz_Idle" => {
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(format!("{}/scripts/config -d TICK_ONESHOT -d TICK_ONESHOT -e TICK_ONESHOT -d TICK_ONESHOT", kernel_src_dir))
                .output()
                .await
                .context("Failed to configure tick type to NoHz_Idle")?;
        }
        _ => {}
    }

    Ok(())
}
