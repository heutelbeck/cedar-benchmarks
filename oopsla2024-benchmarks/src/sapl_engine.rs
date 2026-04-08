use std::io::prelude::*;
use std::io::{BufRead, BufReader, BufWriter};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use cedar_policy_core::ast::Request;
use cedar_policy_core::authorizer::Decision;
use serde::Deserialize;

use crate::entity_graph::EntityGraph;
use crate::sapl_requests::{self, SaplBatch};
use crate::{ExampleApp, SingleExecutionReport};

static JAVA_BINARY: &str = "java";
static SAPL_JAR_PATH: &str = "sapl-harness/target/sapl-harness-1.0.0.jar";

/// Persistent Java process for SAPL evaluation.
/// Spawned once and reused across hierarchy batches.
pub struct SaplProcess {
    child: Child,
    stdin: BufWriter<std::process::ChildStdin>,
    stdout: BufReader<std::process::ChildStdout>,
}

impl SaplProcess {
    pub fn new(app_name: &str) -> Self {
        let mut child = Command::new(JAVA_BINARY)
            .args([
                "-jar",
                SAPL_JAR_PATH,
                "--app",
                app_name,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("failed to spawn SAPL Java process");

        let stdin = BufWriter::new(child.stdin.take().expect("failed to get stdin"));
        let stdout = BufReader::new(child.stdout.take().expect("failed to get stdout"));

        Self {
            child,
            stdin,
            stdout,
        }
    }

    /// Send a batch and receive results.
    fn execute_batch(&mut self, batch: &SaplBatch) -> Vec<SaplTestOutput> {
        let json = serde_json::to_string(batch).expect("failed to serialize SAPL batch");
        writeln!(self.stdin, "{json}").expect("failed to write to SAPL process stdin");
        self.stdin.flush().expect("failed to flush SAPL process stdin");

        let mut line = String::new();
        self.stdout
            .read_line(&mut line)
            .expect("failed to read from SAPL process stdout");

        serde_json::from_str::<Vec<SaplTestOutput>>(&line)
            .unwrap_or_else(|e| panic!("failed to parse SAPL output: {e}\nLine: {line}"))
    }
}

impl Drop for SaplProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SaplTestOutput {
    #[serde(rename = "Decision")]
    decision: bool,
    #[serde(rename = "Dur")]
    dur_nanoseconds: u64,
}

impl From<SaplTestOutput> for SingleExecutionReport {
    fn from(val: SaplTestOutput) -> Self {
        let decision = if val.decision {
            Decision::Allow
        } else {
            Decision::Deny
        };
        SingleExecutionReport {
            dur: Duration::from_nanos(val.dur_nanoseconds),
            decision,
            errors: vec![],
            context_attrs: 0,
        }
    }
}

/// SAPL engine adapter. Holds entity data for one hierarchy.
pub struct SaplEngine<'a, T: EntityGraph> {
    app: &'a ExampleApp,
    entities: T,
}

impl<'a, T: EntityGraph> SaplEngine<'a, T> {
    pub fn new(
        app: &'a ExampleApp,
        entities: impl IntoIterator<Item = &'a cedar_policy_core::ast::Entity>,
    ) -> Self {
        Self {
            app,
            entities: T::from_iter(entities),
        }
    }

    /// Execute requests via the persistent SAPL process.
    pub fn execute(
        &self,
        requests: Vec<Request>,
        process: &mut SaplProcess,
    ) -> impl Iterator<Item = SingleExecutionReport> {
        let batch = match self.app.name {
            "github" | "github-templates" => {
                sapl_requests::github::build_batch(&self.entities, &requests)
            }
            "gdrive" | "gdrive-templates" => {
                sapl_requests::gdrive::build_batch(&self.entities, &requests)
            }
            "tinytodo" => sapl_requests::tinytodo::build_batch(&self.entities, &requests),
            app_name => {
                log::warn!("SAPL engine for {app_name} is not yet implemented");
                return Vec::new().into_iter();
            }
        };

        let results = process.execute_batch(&batch);
        results
            .into_iter()
            .map(|r| r.into())
            .collect::<Vec<_>>()
            .into_iter()
    }
}
