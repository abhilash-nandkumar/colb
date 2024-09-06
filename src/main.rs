use std::{
    ops::Deref,
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
}

struct BasicVerb {
    args: ArgStack,
}

struct ConfiguredBuild {
    args: ArgStack,
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
            args.arg(format!("{workspace}/log"));
        } else {
            args.arg("/dev/null");
        }
        ColconInvocation {
            args,
            workspace: workspace.into(),
        }
    }

    fn build(self, base_setup: &BuildOutput) -> BuildVerb {
        let mut res = BuildVerb { args: self.args };
        res.args.arg("build");
        res.args
            .arg("--build-base")
            .arg(format!("{}/build", self.workspace));
        res.args
            .arg("--install-base")
            .arg(format!("{}/install", self.workspace));
        if base_setup.symlink {
            res.args.arg("--symlink-install");
        }
        if base_setup.merge {
            res.args.arg("--merge-install");
        }
        res
    }

    fn test(self, config: &TestConfiguration) -> BasicVerb {
        let mut res = BasicVerb { args: self.args };
        // TODO: log is probably needed here?
        res.args.arg("test");
        res.args.arg("--event-handlers");
        config.event_handlers.apply(&mut res.args);
        res.args.args(["--ctest-args", "--output-on-failure"]);
        res.args.args(["--packages-select", &config.package]);
        res
    }

    fn test_result(self, config: &TestResultConfig) -> BasicVerb {
        let mut res = BasicVerb { args: self.args };
        // TODO: log is probably needed here?
        res.args.arg("test-result");
        res.args.args([
            "--test-result-base",
            &format!("{}/build/{}", self.workspace, config.package),
        ]);
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
        let mut res = ConfiguredBuild { args: self.args };
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
        /// The package to build
        package: String,

        /// Whether to skip rebuilding dependencies
        #[arg(short, long, default_value_t = false)]
        skip_dependencies: bool,
    },

    /// Run tests for a package
    Test {
        /// The package to test
        package: String,

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
// - auto-detect package
// - test from filename
// - get test stdout (--rerun-failed --output-on-failure)
// - expose more config options
// - persist/load options from disk

fn main() {
    let cli = Cli::parse();
    let ws = cli.workspace.unwrap_or(".".into());
    match &cli.verb {
        Verbs::Build {
            package,
            skip_dependencies,
        } => {
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
                    let status = ninja_build_target(&ws, package, test);
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
                let status = run_single_ctest(&ws, package, test);
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
