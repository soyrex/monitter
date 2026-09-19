use monitter_lib::model_router::{
    evaluate, AgentProvider, CodexCliProvider, JevClassifier, LiveJevClassifier, MockJevClassifier,
    MockProvider, ModelTier, ReasoningLevel, RouterRun, UserOverrides,
};
use std::{env, path::PathBuf, process};

fn usage() -> &'static str {
    "Usage:\n  harness run <prompt> [--workspace <path>] [--provider mock|codex] [--classifier mock|jev] [--model <id>] [--reasoning-level low|medium|high|xhigh] [--tier fast|balanced|strong|frontier] [--max-steps <n>] [--user-correction <note>]\n  harness eval\n\n--classifier jev reads Monitter's Keychain-backed JEV_API_KEY first, then JEV_API_KEY or TYPESAFE_API_KEY from the environment. The codex provider uses the local authenticated Codex CLI in an isolated worktree and never bypasses sandbox or approvals."
}

fn main() {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        fail(usage());
    };
    if command == "eval" {
        println!(
            "{}",
            serde_json::to_string_pretty(&evaluate(&MockJevClassifier))
                .expect("serializable evaluation")
        );
        return;
    }
    if command != "run" {
        fail(usage());
    }
    let Some(prompt) = args.next() else {
        fail(usage());
    };

    let mut workspace = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut provider = "mock".to_string();
    let mut classifier_name = "mock".to_string();
    let mut max_steps = 6_u32;
    let mut overrides = UserOverrides::default();
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .unwrap_or_else(|| fail(&format!("Missing value for {flag}\n\n{}", usage())));
        match flag.as_str() {
            "--workspace" => workspace = PathBuf::from(value),
            "--provider" => provider = value,
            "--classifier" => classifier_name = value,
            "--model" => overrides.model = Some(value),
            "--reasoning-level" => overrides.reasoning_level = Some(parse_reasoning(&value)),
            "--tier" => overrides.preferred_tier = Some(parse_tier(&value)),
            "--max-steps" => {
                max_steps = value
                    .parse()
                    .unwrap_or_else(|_| fail("--max-steps must be a positive integer"))
            }
            "--user-correction" => overrides.user_correction = Some(value),
            _ => fail(&format!("Unknown option: {flag}\n\n{}", usage())),
        }
    }
    if max_steps == 0 {
        fail("--max-steps must be greater than zero");
    }
    let provider: Box<dyn AgentProvider> = match provider.as_str() {
        "mock" => Box::new(MockProvider),
        "codex" => Box::new(CodexCliProvider::from_env().unwrap_or_else(|error| fail(&error))),
        _ => fail("--provider must be mock or codex"),
    };
    let classifier: Box<dyn JevClassifier> = match classifier_name.as_str() {
        "mock" => Box::new(MockJevClassifier),
        "jev" => Box::new(
            LiveJevClassifier::from_monitter_secret()
                .or_else(|_| LiveJevClassifier::from_env())
                .unwrap_or_else(|error| fail(&error)),
        ),
        _ => fail("--classifier must be mock or jev"),
    };
    let trace = RouterRun {
        provider: provider.as_ref(),
        classifier: classifier.as_ref(),
        workspace: &workspace,
        max_steps,
        file_budget: 8,
        overrides,
    }
    .execute(&prompt)
    .unwrap_or_else(|error| fail(&error));
    println!(
        "{}",
        serde_json::to_string_pretty(&trace).expect("serializable trace")
    );
}

fn parse_reasoning(value: &str) -> ReasoningLevel {
    match value {
        "low" => ReasoningLevel::Low,
        "medium" => ReasoningLevel::Medium,
        "high" => ReasoningLevel::High,
        "xhigh" => ReasoningLevel::Xhigh,
        _ => fail("--reasoning-level must be low, medium, high, or xhigh"),
    }
}

fn parse_tier(value: &str) -> ModelTier {
    match value {
        "fast" => ModelTier::Fast,
        "balanced" => ModelTier::Balanced,
        "strong" => ModelTier::Strong,
        "frontier" => ModelTier::Frontier,
        _ => fail("--tier must be fast, balanced, strong, or frontier"),
    }
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    process::exit(2);
}
