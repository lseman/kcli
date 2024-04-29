# capyCachy Kernel Manager

`capyCachy Kernel Manager` is a comprehensive tool designed for managing Linux kernels, specifically tailored for handling CachyOS cherry-picked kernel patches. It's built using Rust for robust performance and reliability. This tool allows users to download both stable and git versions of the kernel, apply patches from the CachyOS kernel repository, and manage these kernels using an integrated package manager.

## Features

- **Download Kernel Source:** Support for both stable releases and latest git snapshots.
- **Patch Kernel:** Apply patches directly from the CachyOS repository.
- **Build Kernel:** Compile your kernel right from the source.
- **Package Kernel:** Package your custom-built kernel for easier installation and distribution.
- **Configure Kernel Options:** Tailor your kernel configuration to best fit your needs.
- **Advanced Search/Configure:** Advanced tools for seasoned users who wish to fine-tune their kernel installations.

## Installation

To get started with `capyCachy Kernel Manager`, clone this repository to your local machine:

```bash
git clone https://github.com/lseman/kcli.git
cd capycachy-kernel-manager
```

Follow the instructions below to compile from source:

```bash
# Add additional commands if needed
cargo build --release
```

## Usage

To launch the kernel manager, navigate to the directory where you cloned the repository and run:

```bash
cargo run
```

### Kernel Configuration Menu

```
? Kernel Configuration Menu ›
❯ Download Kernel Source
  Configure Kernel Options
  Patch Kernel
  Build Kernel
  Package Kernel
  Advanced Search/Configure
  Exit
```

Select the desired option by navigating with your keyboard arrows and pressing Enter.

## Contributing

Contributions are welcome! For major changes, please open an issue first to discuss what you would like to change.

## License

[MIT](https://choosealicense.com/licenses/mit/)