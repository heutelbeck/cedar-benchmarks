use cedar_benchmarks::CedarOptEngine;
use cedar_benchmarks::{
    CedarEngine, Engine, ExampleApp, HierarchyStats, MultiExecutionReport, OpenFgaEngine,
    RandomBytes, RegoEngine,
};
use cedar_benchmarks::sapl_engine::{SaplEngine, SaplProcess};
use cedar_benchmarks::OpenEntities;
use cedar_policy_core::ast::Request;
use cedar_policy_core::entities::{Entities, TCComputation};
use cedar_policy_core::extensions::Extensions;
use cedar_policy_core::validator::CoreSchema;
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::process::Command;

#[derive(Parser, Debug)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run benchmarks
    Bench(BenchArgs),
}

#[derive(Args, Debug)]
pub struct BenchArgs {
    #[clap(flatten)]
    common_args: CommonBenchArgs,

    /// Number of bytes to use for `Unstructured`
    ///
    /// To run multiple experiments, use comma-separated values.
    /// E.g., `--unstructured-bytes 1024,2048,4096` runs three experiments, one
    /// with each of the values, and generates plots accordingly.
    #[arg(short, long, value_delimiter = ',', default_value = "8192")]
    unstructured_bytes: Vec<usize>,

    /// Number of entities to generate, per entity type.
    /// E.g., `--num-entities 3` generates a hierarchy with three entities per entity type.
    ///
    /// To run multiple experiments, use comma-separated values.
    /// E.g., `--num-entities 3,4,5` runs three experiments, one with each of the
    /// values of the num-entities parameter, and generates plots accordingly.
    #[arg(long, value_delimiter = ',', default_value = "3")]
    num_entities: Vec<usize>,
}

/// Parameters which we don't experimentally vary
#[derive(Args, Debug)]
pub struct CommonBenchArgs {
    /// Which example application to benchmark
    ///
    /// To run multiple applications, use comma-separate values.
    /// E.g., `--app github,gdrive` runs all experiments with both the `github`
    /// and `gdrive` apps.
    #[arg(long, value_enum, value_delimiter = ',', required = true)]
    app: Vec<App>,

    /// Which authorization engine to test
    ///
    /// Comma-separated values are accepted.
    /// E.g., `--engine cedar,openfga` performs all tests both on Cedar and on
    /// OpenFGA.
    #[arg(long, value_enum, value_delimiter = ',', default_value = "cedar")]
    engine: Vec<EngineChoice>,

    /// Number of requests to generate, per hierarchy
    #[arg(short = 'r', long, default_value_t = 100)]
    num_requests: usize,

    /// Number of hierarchies to generate, per experiment.
    /// Each experiment will generate this many hierarchies, and `num-requests` requests per hierarchy.
    #[arg(short = 'n', long, default_value_t = 100)]
    num_hierarchies: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum, Hash)]
pub enum App {
    /// Github example application
    Github,
    /// Github example application, alternate templates encoding
    GithubTemplates,
    /// Gdrive example application
    Gdrive,
    /// Gdrive example application, alternate templates encoding
    GdriveTemplates,
    /// TinyTodo example application
    TinyTodo,
}

impl std::fmt::Display for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Github => write!(f, "github"),
            Self::GithubTemplates => write!(f, "github-templates"),
            Self::Gdrive => write!(f, "gdrive"),
            Self::GdriveTemplates => write!(f, "gdrive-templates"),
            Self::TinyTodo => write!(f, "tinytodo"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, ValueEnum)]
pub enum EngineChoice {
    /// Cedar authorization engine
    Cedar,
    /// OpenFGA authorization engine
    OpenFGA,
    /// Cedar with policy slicing
    CedarOpt,
    /// Rego authorization engine
    Rego,
    /// Rego authorization engine, pre-compute transitive closure
    RegoPreTC,
    /// SAPL authorization engine (embedded JVM via subprocess)
    Sapl,
}

impl std::fmt::Display for EngineChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cedar => write!(f, "cedar"),
            Self::OpenFGA => write!(f, "openfga"),
            Self::CedarOpt => write!(f, "cedaropt"),
            Self::Rego => write!(f, "rego"),
            Self::RegoPreTC => write!(f, "rego_pre_tc"),
            Self::Sapl => write!(f, "sapl"),
        }
    }
}

fn main() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();
    let cli = Cli::parse();
    match cli.command {
        Commands::Bench(args) => bench(args),
    }
}

/// Run a single experiment.
/// This involves generating potentially many hierarchies, and many requests per hierarchy,
/// but with constant num-entities and unstructured-bytes parameters.
fn run_experiment(
    app: App,
    num_entities: usize,
    unstructured_bytes: usize,
    common_args: &CommonBenchArgs,
    bytes: &mut RandomBytes,
) -> (HierarchyStats, BTreeMap<EngineChoice, MultiExecutionReport>) {
    eprintln!("Running experiment for {app} with {unstructured_bytes} unstructured bytes and {num_entities} entities per entity type...");
    let app = match app {
        App::Github => ExampleApp::github(&mut bytes.unstructured(unstructured_bytes)),
        App::GithubTemplates => {
            ExampleApp::github_templates(&mut bytes.unstructured(unstructured_bytes))
        }
        App::Gdrive => ExampleApp::gdrive(&mut bytes.unstructured(unstructured_bytes)),
        App::GdriveTemplates => {
            ExampleApp::gdrive_templates(&mut bytes.unstructured(unstructured_bytes))
        }
        App::TinyTodo => ExampleApp::tinytodo(&mut bytes.unstructured(unstructured_bytes)),
    };
    let mut hierarchy_stats = HierarchyStats::new();
    let mut multireports: BTreeMap<EngineChoice, MultiExecutionReport> = BTreeMap::new();
    // SAPL process persists across hierarchies for JVM warmup amortization
    let use_sapl = common_args.engine.contains(&EngineChoice::Sapl);
    let mut sapl_process = if use_sapl {
        Some(SaplProcess::new(&app.name))
    } else {
        None
    };
    (1..=common_args.num_hierarchies).for_each(|_| {
        let (entities, links) = (app.bespoke_generator)(&app.validator_schema(), num_entities);
        let mut num_openfga_tuples = 0;
        let engines: Vec<_> = common_args.engine.iter().filter_map(|choice| match choice {
            EngineChoice::Cedar => {
                let engine = CedarEngine::new(&entities, &links, &app);
                Some((choice, Engine::Cedar(engine)))
            },
            EngineChoice::CedarOpt => {
                let engine = CedarOptEngine::new(&entities, &links, &app);
                Some((choice, Engine::CedarOpt(engine)))
            }
            EngineChoice::OpenFGA => {
                let engine = OpenFgaEngine::new(&entities, links.clone(), &app);
                num_openfga_tuples = engine.num_tuples();
                Some((choice, Engine::OpenFga(engine)))
            },
            EngineChoice::Rego => {
                let engine = RegoEngine::new(&app, &entities);
                Some((choice, Engine::Rego(engine)))
            },
            EngineChoice::RegoPreTC => {
                let engine = RegoEngine::new(&app, &entities);
                Some((choice, Engine::RegoTC(engine)))
            },
            EngineChoice::Sapl => None, // handled separately below
        }).collect();
        let cedar_entities = Entities::from_entities(
            entities.clone(),
            Some(&CoreSchema::new(&app.validator_schema())),
            TCComputation::ComputeNow,
            Extensions::all_available(),
        ).unwrap();
        let requests: Vec<Request> = {
            let mut vec = Vec::with_capacity(common_args.num_requests);
            let u = &mut bytes.unstructured(unstructured_bytes);
            let hierarchy = cedar_entities.clone().into();
            let mut generate_request = || {
                app.schema
                    .arbitrary_request(&hierarchy, u)
                    .expect("failed to generate request")
                    .into()
            };
            vec.resize_with(common_args.num_requests, || {
                // `arbitrary_request()` intentionally sometimes generates requests with nonexistent
                // principal and/or resource.
                // We don't want that behavior here, but rather than change the behavior of the
                // request generator, we just keep calling it until we get one we want.
                loop {
                    let request: Request = generate_request();
                    if hierarchy.uids().iter().any(|h_uid| request.principal().uid() == Some(h_uid))
                        && hierarchy.uids().iter().any(|h_uid| request.resource().uid() == Some(h_uid))
                        {
                            break request;
                        }
                }
            });
            vec
        };
        // The "decision sequence" of each engine is the allow/deny sequence, which we assert needs to be the same for each engine else we're doing something wrong. (The engines should have the same allow/deny behavior.)
        let mut decision_sequences: Vec<Vec<_>> = Vec::new();
        let mut engine_names: Vec<EngineChoice> = Vec::new();
        for (choice, engine) in engines {
            let mreport = multireports.entry(*choice).or_insert_with(MultiExecutionReport::new);
            decision_sequences.push(Vec::with_capacity(requests.len()));
            engine_names.push(*choice);
            let decision_sequence = decision_sequences.last_mut().expect("just pushed, so there should be a last element");
            for sreport in engine.execute(requests.clone()) {
                decision_sequence.push(sreport.decision);
                mreport.add(sreport);
            }
        }
        // SAPL engine: uses persistent process, executed after other engines
        if let Some(ref mut process) = sapl_process {
            let sapl_engine: SaplEngine<'_, OpenEntities> = SaplEngine::new(&app, &entities);
            let mreport = multireports.entry(EngineChoice::Sapl).or_insert_with(MultiExecutionReport::new);
            decision_sequences.push(Vec::with_capacity(requests.len()));
            engine_names.push(EngineChoice::Sapl);
            let decision_sequence = decision_sequences.last_mut().unwrap();
            for sreport in sapl_engine.execute(requests.clone(), process) {
                decision_sequence.push(sreport.decision);
                mreport.add(sreport);
            }
        }
        // Validate all engines agree on decisions
        if decision_sequences.len() >= 2 {
            for (req_idx, request) in requests.iter().enumerate() {
                let first_decision = &decision_sequences[0][req_idx];
                for seq_idx in 1..decision_sequences.len() {
                    if decision_sequences[seq_idx][req_idx] != *first_decision {
                        eprintln!("=== DECISION MISMATCH ===");
                        for (i, req) in requests.iter().enumerate() {
                            let decisions: Vec<_> = decision_sequences.iter()
                                .map(|seq| format!("{:?}", seq[i]))
                                .collect();
                            let marker = if i == req_idx { " <-- MISMATCH" } else { "" };
                            eprintln!("  req[{}] {} -> [{}]{}", i, req, decisions.join(", "), marker);
                        }
                        panic!(
                            "Decision mismatch for {request}: {} gave {first_decision:?} but {} gave {:?}",
                            engine_names[0], engine_names[seq_idx], decision_sequences[seq_idx][req_idx]
                        );
                    }
                }
            }
        } else if decision_sequences.is_empty() {
            panic!("expected at least 1 decision sequence");
        }
        hierarchy_stats.add(&cedar_entities, num_openfga_tuples);
    });
    //multireport.print(&mut std::io::stderr()).unwrap();
    (hierarchy_stats, multireports)
}

fn bench(args: BenchArgs) {
    let common_args = &args.common_args;
    let num_entities = &args.num_entities;
    let unstructured_bytes = &args.unstructured_bytes[..];
    std::fs::create_dir_all("output/").expect("failed to create output directory");
    for app in &common_args.app {
        let mut multireports = num_entities
            .iter()
            .flat_map(move |num_entities| {
                let mut bytes = RandomBytes::new();
                unstructured_bytes.iter().map(move |unstructured_bytes| {
                    let (hstats, multireports) = run_experiment(
                        *app,
                        *num_entities,
                        *unstructured_bytes,
                        common_args,
                        &mut bytes,
                    );
                    (num_entities, unstructured_bytes, hstats, multireports)
                })
            })
            .peekable();
        let csv_filename = match app {
            App::Github => "output/github.csv",
            App::GithubTemplates => "output/github-templates.csv",
            App::Gdrive => "output/gdrive.csv",
            App::GdriveTemplates => "output/gdrive-templates.csv",
            App::TinyTodo => "output/tinytodo.csv",
        };
        let mut csv = File::create(csv_filename).expect("failed to create output CSV");
        write!(&mut csv, "num_entities,unstructured_bytes,").unwrap();
        for choice in common_args.engine.iter() {
            MultiExecutionReport::csv_header(&mut csv, &choice.to_string()).unwrap();
            write!(&mut csv, ",").unwrap();
        }
        {
            // to write the hierarchy stats portion of the header we need a concrete `HierarchyStats` object,
            // so we just use the first one and assume it's representative
            let Some((_, _, hstats, _)) = multireports.peek() else {
                eprintln!("this combination of args resulted in no experiments to run");
                return;
            };
            hstats.csv_header(&mut csv).unwrap();
        }
        writeln!(&mut csv).unwrap();
        for (num_entities, unstructured_bytes, hierarchy_stats, multireports) in multireports {
            write!(&mut csv, "{num_entities},{unstructured_bytes},").unwrap();
            for engine in &common_args.engine {
                let multireport = multireports.get(engine).unwrap();
                multireport.csv_row(&mut csv).unwrap();
                write!(&mut csv, ",").unwrap();
            }
            hierarchy_stats.csv_row(&mut csv).unwrap();
            writeln!(&mut csv).unwrap();
        }
    }
    eprintln!("generating plot(s)...");
    Command::new("python3").arg("./plot.py").status().unwrap();
    eprintln!("Done")
}
