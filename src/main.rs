use std::{
    env,
    ops::Deref,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

use clap::{Parser, Subcommand};

enum BuildType {
    Debug,
    Release,
    RelWithDebInfo,
}

impl BuildType {
    fn apply(&self, cmd: &mut ArgStack) {
        cmd.arg("--cmake-args");
        let t = match self {
            BuildType::Debug => "Debug",
            BuildType::Release => "Release",
            BuildType::RelWithDebInfo => "RelWithDebInfo",
        };
        cmd.arg(format!("-DCMAKE_BUILD_TYPE={t}"));
    }
}

#[derive(Default)]
struct ArgStack {
    args: Vec<String>,
}

impl ArgStack {
    pub fn arg<S: Into<String>>(&mut self, arg: S) -> &mut Self {
        self.args.push(arg.into());
        self
    }

    fn args<I, S>(&mut self, args: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for arg in args {
            self.arg(arg.into());
        }
    }
}

// Make ArgStack.iter() work
impl Deref for ArgStack {
    type Target = Vec<String>;

    fn deref(&self) -> &Self::Target {
        &self.args
    }
}

struct ColconInvocation {
    args: ArgStack,
    workspace: String,
}

struct BuildVerb {
    args: ArgStack,
    workspace: String,
}

struct BasicVerb {
    args: ArgStack,
    workspace: String,
}

struct ConfiguredBuild {
    args: ArgStack,
    workspace: String,
}

#[derive(Default)]
struct BuildOutput {
    symlink: bool,
    merge: bool,
}

struct EventHandlers {
    desktop_notification: bool,
    console_cohesion: bool,
    summary: bool,
    console_start_end: bool,
}

impl Default for EventHandlers {
    fn default() -> Self {
        Self {
            desktop_notification: false,
            console_cohesion: false,
            summary: true,
            console_start_end: true,
        }
    }
}

impl EventHandlers {
    fn silent() -> Self {
        Self {
            desktop_notification: false,
            console_cohesion: false,
            summary: false,
            console_start_end: false,
        }
    }

    fn compile_logs_only() -> Self {
        let mut res = Self::silent();
        res.console_cohesion = true;
        res
    }

    fn apply(&self, args: &mut ArgStack) {
        args.arg("--event-handlers");
        args.arg(handler_str("summary", self.summary));
        args.arg(handler_str("console_start_end", self.console_start_end));
        args.arg(handler_str("console_cohesion", self.console_cohesion));
        args.arg(handler_str(
            "desktop_notification",
            self.desktop_notification,
        ));
    }
}

struct BuildConfiguration {
    mixins: Vec<String>,
    cmake_args: Vec<String>,
    build_type: BuildType,
    parallel_jobs: Option<u32>,
    event_handlers: EventHandlers,
    build_tests: bool,
}

struct TestConfiguration {
    package: String,
    event_handlers: EventHandlers,
}

struct TestResultConfig {
    package: String,
    verbose: bool,
    all: bool,
}

enum What {
    DependenciesFor(String),
    ThisPackage(String),
}

impl ColconInvocation {
    fn new(workspace: &str, log: bool) -> ColconInvocation {
        let mut args = ArgStack::default();
        args.arg("--log-base");
        if log {
            args.arg("log");
        } else {
            args.arg("/dev/null");
        }
        ColconInvocation {
            args,
            workspace: workspace.into(),
        }
    }

    fn build(self, base_setup: &BuildOutput) -> BuildVerb {
        let mut res = BuildVerb {
            args: self.args,
            workspace: self.workspace,
        };
        res.args.arg("build");
        res.args
            .args(["--build-base", "build", "--install-base", "install"]);
        if base_setup.symlink {
            res.args.arg("--symlink-install");
        }
        if base_setup.merge {
            res.args.arg("--merge-install");
        }
        res
    }

    fn test(self, config: &TestConfiguration) -> BasicVerb {
        let mut res = BasicVerb {
            args: self.args,
            workspace: self.workspace,
        };
        // TODO: log is probably needed here?
        res.args.arg("test");
        res.args.arg("--event-handlers");
        config.event_handlers.apply(&mut res.args);
        res.args.args(["--ctest-args", "--output-on-failure"]);
        res.args.args(["--packages-select", &config.package]);
        res
    }

    fn test_result(self, config: &TestResultConfig) -> BasicVerb {
        let mut res = BasicVerb {
            args: self.args,
            workspace: self.workspace,
        };
        // TODO: log is probably needed here?
        res.args.arg("test-result");
        res.args
            .args(["--test-result-base", &format!("build/{}", config.package)]);
        if config.verbose {
            res.args.arg("--verbose");
        }
        if config.all {
            res.args.arg("--all");
        }
        res
    }
}

fn handler_str(name: &str, enabled: bool) -> String {
    format!("{name}{}", if enabled { "+" } else { "-" })
}

fn cmake_arg(name: &str, value: &str) -> String {
    format!("-D{name}={value}")
}

impl BuildConfiguration {
    const DEFAULT_MIXINS: &'static [&'static str] =
        &["compile-commands", "ninja", "mold", "ccache"];
    fn upstream() -> BuildConfiguration {
        BuildConfiguration {
            mixins: Self::DEFAULT_MIXINS
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<String>>(),
            cmake_args: vec![],
            build_type: BuildType::Debug,
            parallel_jobs: Some(8),
            event_handlers: EventHandlers::default(),
            build_tests: false,
        }
    }

    fn active() -> BuildConfiguration {
        BuildConfiguration {
            mixins: Self::DEFAULT_MIXINS
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<String>>(),
            cmake_args: vec![],
            build_type: BuildType::Debug,
            parallel_jobs: Some(8),
            event_handlers: EventHandlers::compile_logs_only(),
            build_tests: true,
        }
    }

    fn for_testing() -> BuildConfiguration {
        // TODO: add summary- and console_start_end-
        BuildConfiguration {
            mixins: Self::DEFAULT_MIXINS
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<String>>(),
            cmake_args: vec![],
            build_type: BuildType::Debug,
            parallel_jobs: Some(8),
            event_handlers: EventHandlers::compile_logs_only(),
            build_tests: true,
        }
    }
}

impl BuildVerb {
    fn configure(self, config: &BuildConfiguration) -> ConfiguredBuild {
        let mut res = ConfiguredBuild {
            args: self.args,
            workspace: self.workspace,
        };
        if let Some(n) = config.parallel_jobs {
            let n_arg = format!("{}", n);
            res.args
                .args(["--executor", "parallel", "--parallel-workers", &n_arg]);
        }
        config.event_handlers.apply(&mut res.args);
        if !config.mixins.is_empty() {
            res.args.arg("--mixin").args(config.mixins.iter());
        }
        res.args.arg("--cmake-args");
        res.args.arg(cmake_arg(
            "BUILD_TESTING",
            if config.build_tests { "ON" } else { "OFF" },
        ));
        res.args.args(config.cmake_args.iter());
        config.build_type.apply(&mut res.args);
        res
    }
}

macro_rules! header {
    ($($l:tt)*) => {
        print!("┌[ ");
        print!($($l)*);
        println!(" ]");
    };
}

fn log_command(command: &Command) {
    print!("└> {}", command.get_program().to_string_lossy());
    for arg in command.get_args() {
        print!(" {}", arg.to_string_lossy());
    }
    println!();
}

impl ConfiguredBuild {
    fn run(&self, what: &What) -> ExitStatus {
        let mut cmd = Command::new("colcon");
        cmd.current_dir(&self.workspace);
        cmd.args(self.args.iter());
        match what {
            What::DependenciesFor(package) => {
                cmd.arg("--packages-up-to").arg(package);
                cmd.arg("--packages-skip").arg(package);
            }
            What::ThisPackage(package) => {
                cmd.arg("--packages-select").arg(package);
            }
        }
        log_command(&cmd);
        cmd.status().expect("'colcon' not found")
    }
}

impl BasicVerb {
    fn run(&self) -> ExitStatus {
        let mut cmd = Command::new("colcon");
        cmd.current_dir(&self.workspace);
        cmd.args(self.args.iter());
        log_command(&cmd);
        cmd.status().expect("'colcon' not found")
    }
}

fn ninja_build_target(workspace: &str, package: &str, target: &str) -> ExitStatus {
    let mut cmd = Command::new("ninja");
    cmd.arg("-C");
    cmd.arg(format!("{workspace}/build/{package}"));
    cmd.arg(target);
    log_command(&cmd);
    cmd.status().expect("'ninja' not found")
}

fn run_single_ctest(workspace: &str, package: &str, target: &str) -> ExitStatus {
    let mut cmd = Command::new("ctest");
    cmd.arg("--test-dir");
    cmd.arg(format!("{workspace}/build/{package}"));
    // cmd.arg("--output-on-failure");
    cmd.arg("-R");
    cmd.arg(format!("^{target}$"));
    log_command(&cmd);
    cmd.status().expect("'ctest' not found")
}

fn contains_marker(path: &Path, marker: &str) -> Option<PathBuf> {
    let candidate = path.join(marker);
    match candidate.try_exists() {
        Ok(true) => Some(path.to_path_buf()),
        _ => None,
    }
}

/// Search upward, and if we hit a package.xml, use that folder name as the package
fn find_upwards(marker: &str) -> Option<PathBuf> {
    let mut cwd = env::current_dir().and_then(|p| p.canonicalize()).ok()?;
    let mut res = contains_marker(&cwd, marker);
    while res.is_none() {
        cwd = cwd.parent().map(|x| x.to_path_buf())?;
        res = contains_marker(&cwd, marker);
    }
    res
}

fn package_or(package: Option<String>) -> Option<String> {
    if package.is_some() {
        return package;
    }
    find_upwards("package.xml").and_then(|f| f.file_name().map(|n| n.to_string_lossy().to_string()))
}

fn detect_workspace() -> Option<String> {
    find_upwards("build").map(|n| n.to_string_lossy().to_string())
}

/// A colcon wrapper for faster change compile test cycles
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[arg(short, long)]
    workspace: Option<String>,

    #[command(subcommand)]
    verb: Verbs,
}

#[derive(Subcommand)]
enum Verbs {
    /// Build a package
    Build {
        /// The package to build (default: current directory)
        package: Option<String>,

        /// Whether to skip rebuilding dependencies
        #[arg(short, long, default_value_t = false)]
        skip_dependencies: bool,
    },

    /// Run tests for a package
    Test {
        /// The package to test (default: current directory)
        package: Option<String>,

        /// Build and run only this test (default: run all tests)
        #[arg(short, long)]
        test: Option<String>,

        /// Don't rebuild the package
        #[arg(short, long, default_value_t = false)]
        skip_rebuild: bool,

        /// Rebuild dependencies of package
        #[arg(short, long, default_value_t = false)]
        rebuild_dependencies: bool,
    },
}

fn exit_on_error(status: ExitStatus) {
    match status.code() {
        Some(0) => {}
        Some(code) => {
            std::process::exit(code);
        }
        None => {
            std::process::exit(-1);
        }
    }
}

// TODOs:
// - expose more config options
// - persist/load options from disk

fn main() {
    let exit_on_not_found = || {
        eprintln!("Could not detect package, try specifying it explicitly!");
        std::process::exit(-1);
    };

    let cli = Cli::parse();
    let ws = cli
        .workspace
        .or_else(detect_workspace)
        .unwrap_or(".".into());
    println!(
        "┌[ Workspace ]\n└> {}",
        Path::new(&ws)
            .canonicalize()
            .map(|x| x.to_string_lossy().to_string())
            .unwrap_or(ws.clone())
    );
    match &cli.verb {
        Verbs::Build {
            package,
            skip_dependencies,
        } => {
            let package = package_or(package.clone())
                .or_else(exit_on_not_found)
                .expect("should have exited");
            if !skip_dependencies {
                header!("Building dependencies");
                let status = ColconInvocation::new(&ws, false)
                    .build(&BuildOutput::default())
                    .configure(&BuildConfiguration::upstream())
                    .run(&What::DependenciesFor(package.clone()));
                exit_on_error(status);
            }
            header!("Building '{package}'");
            let status = ColconInvocation::new(&ws, false)
                .build(&BuildOutput::default())
                .configure(&BuildConfiguration::active())
                .run(&What::ThisPackage(package.clone()));
            exit_on_error(status);
        }

        Verbs::Test {
            package,
            test,
            skip_rebuild,
            rebuild_dependencies,
        } => {
            let package = package_or(package.clone())
                .or_else(exit_on_not_found)
                .expect("should have exited");
            if *rebuild_dependencies && !skip_rebuild {
                header!("Building dependencies");
                let status = ColconInvocation::new(&ws, false)
                    .build(&BuildOutput::default())
                    .configure(&BuildConfiguration::upstream())
                    .run(&What::DependenciesFor(package.clone()));
                exit_on_error(status);
                if test.is_some() {
                    header!("Building '{package}'");
                    let status = ColconInvocation::new(&ws, false)
                        .build(&BuildOutput::default())
                        .configure(&BuildConfiguration::active())
                        .run(&What::ThisPackage(package.clone()));
                    exit_on_error(status);
                }
            }
            if !skip_rebuild {
                if let Some(test) = test {
                    header!("Building test '{test}' in '{package}'");
                    let status = ninja_build_target(&ws, &package, test);
                    exit_on_error(status);
                } else {
                    header!("Building '{package}'");
                    let status = ColconInvocation::new(&ws, false)
                        .build(&BuildOutput::default())
                        .configure(&BuildConfiguration::for_testing())
                        .run(&What::ThisPackage(package.clone()));
                    exit_on_error(status);
                }
            }
            if let Some(test) = test {
                header!("Running test '{test}' in '{package}'");
                let status = run_single_ctest(&ws, &package, test);
                exit_on_error(status);
            } else {
                header!("Running tests for '{package}'");
                let status = ColconInvocation::new(&ws, true)
                    .test(&TestConfiguration {
                        package: package.clone(),
                        event_handlers: EventHandlers::silent(),
                    })
                    .run();
                exit_on_error(status);
                header!("Test results for '{package}'");
                let status = ColconInvocation::new(&ws, false)
                    .test_result(&TestResultConfig {
                        package: package.clone(),
                        verbose: true,
                        all: true,
                    })
                    .run();
                exit_on_error(status);
            }
        }
    }
}
