//! Pluggable local model backends.
//!
//! The mock is deterministic and is what tests and CI use. The local
//! backend speaks the OpenAI chat-completions JSON that Ollama and
//! llama.cpp serve on loopback. Anything that is not a loopback host is
//! refused. There is no cloud fallback and no telemetry.

use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::error::{err, Result};
use crate::genome::{Floor, Genes, LawKind};
use crate::http;
use crate::plant::{DT, K_COOL, K_FLOW, K_HEAT, K_LEAK, K_LOSS};

#[derive(Clone, Debug)]
pub struct MutationRequest {
    pub generation: u32,
    pub law: LawKind,
    pub genes: Genes,
    pub fitness: f64,
    pub source: String,
    pub diagnostics: String,
}

#[derive(Clone, Debug)]
pub struct GeneEdit {
    pub name: &'static str,
    pub value: f64,
    pub floor: Floor,
}

#[derive(Clone, Debug)]
pub struct MutationResponse {
    pub law: Option<LawKind>,
    pub edits: Vec<GeneEdit>,
    pub note: String,
}

pub trait LlmBackend {
    fn name(&self) -> &str;
    fn mutate(&self, request: &MutationRequest) -> Result<MutationResponse>;
}

/// Deterministic stand-in for the structural mutation operator.
///
/// It walks the elite toward integral action, then feedforward, then a
/// one-step predictor. Numeric search does the tuning. The same generation
/// and law always produce the same proposal.
#[derive(Clone, Debug)]
pub struct MockBackend;

impl LlmBackend for MockBackend {
    fn name(&self) -> &str {
        "mock"
    }

    fn mutate(&self, request: &MutationRequest) -> Result<MutationResponse> {
        Ok(mock_proposal(request.generation, request.law))
    }
}

pub fn mock_proposal(generation: u32, law: LawKind) -> MutationResponse {
    if generation >= 1 && law == LawKind::Proportional {
        return MutationResponse {
            law: Some(LawKind::Pi),
            edits: vec![GeneEdit {
                name: "ki",
                value: 0.02,
                floor: Floor::AtLeast,
            }],
            note: "enable integral action; proportional control leaves a steady offset".into(),
        };
    }
    if generation >= 5 && law == LawKind::Pi {
        return MutationResponse {
            law: Some(LawKind::PiFeedforward),
            edits: vec![
                GeneEdit {
                    name: "k_amb",
                    value: K_LOSS / K_HEAT,
                    floor: Floor::AtLeast,
                },
                GeneEdit {
                    name: "k_flow",
                    value: K_FLOW / K_HEAT,
                    floor: Floor::AtLeast,
                },
                GeneEdit {
                    name: "k_leak",
                    value: -K_LEAK / K_HEAT,
                    floor: Floor::AtMost,
                },
                GeneEdit {
                    name: "k_cool",
                    value: 0.2,
                    floor: Floor::AtLeast,
                },
            ],
            note: "add feedforward for ambient, flow, and the leak, plus a coolant valve".into(),
        };
    }
    if generation >= 8 && law == LawKind::PiFeedforward {
        return MutationResponse {
            law: Some(LawKind::PiFeedforwardPredict),
            edits: vec![
                GeneEdit {
                    name: "pred_heat",
                    value: DT * K_HEAT,
                    floor: Floor::AtLeast,
                },
                GeneEdit {
                    name: "pred_loss",
                    value: DT * K_LOSS,
                    floor: Floor::AtLeast,
                },
                GeneEdit {
                    name: "pred_flow",
                    value: DT * K_FLOW,
                    floor: Floor::AtLeast,
                },
                GeneEdit {
                    name: "pred_leak",
                    value: DT * K_LEAK,
                    floor: Floor::AtLeast,
                },
                GeneEdit {
                    name: "pred_cool",
                    value: DT * K_COOL,
                    floor: Floor::AtLeast,
                },
                GeneEdit {
                    name: "mix",
                    value: 0.0,
                    floor: Floor::Set,
                },
            ],
            note: "attach a one-step linear predictor; mix stays at 0 so feedback is unchanged"
                .into(),
        };
    }
    MutationResponse {
        law: None,
        edits: Vec::new(),
        note: "no structural change".into(),
    }
}

pub fn apply_proposal(law: &mut LawKind, genes: &mut Genes, proposal: &MutationResponse) {
    if let Some(next) = proposal.law {
        *law = next;
    }
    for edit in &proposal.edits {
        let _ = genes.floor_named(edit.name, edit.value, edit.floor);
    }
}

#[derive(Clone, Debug)]
pub struct LocalBackend {
    endpoint: String,
    model: String,
    timeout: Duration,
}

impl LocalBackend {
    pub fn new(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        timeout: Duration,
    ) -> Result<Self> {
        let endpoint = normalize_endpoint(endpoint.into())?;
        Ok(Self {
            endpoint,
            model: model.into(),
            timeout,
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

impl LlmBackend for LocalBackend {
    fn name(&self) -> &str {
        "local"
    }

    fn mutate(&self, request: &MutationRequest) -> Result<MutationResponse> {
        let content = chat(
            &self.endpoint,
            &self.model,
            SYSTEM_MUTATE,
            &mutation_prompt(request),
            self.timeout,
        )?;
        parse_mutation(&content)
    }
}

pub fn normalize_endpoint(raw: impl AsRef<str>) -> Result<String> {
    let raw = raw.as_ref().trim().trim_end_matches('/');
    let Some(rest) = raw.strip_prefix("http://") else {
        return Err(err(
            "the model endpoint must be http:// on a loopback host (https and non-local hosts are refused)",
        ));
    };
    let (host_port, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
    };
    if host_port.is_empty() {
        return Err(err("the model endpoint is missing a host"));
    }
    let host = if let Some(host) = host_port.strip_prefix('[') {
        host.split(']').next().unwrap_or("")
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    if !is_loopback(host) {
        return Err(err(format!(
            "refusing model endpoint host `{host}`: Crab Cage only talks to a local model"
        )));
    }
    let path = if path.is_empty() { "/v1" } else { path };
    Ok(format!("http://{host_port}{path}"))
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

fn mutation_prompt(request: &MutationRequest) -> String {
    format!(
        "generation: {}\ncurrent law: {}\nfitness (lower is better): {:.6}\n\
         genes: {}\ndiagnostics:\n{}\n\n\
         Reply with one JSON object and nothing else. Fields: \
         \"law\" (hold, proportional, pi, pid, pi_feedforward, pi_feedforward_predict), \
         \"genes\" (object of numeric overrides), \"note\" (one sentence). \
         Do not invent inputs or outputs. The host emits the Rust.",
        request.generation,
        request.law,
        request.fitness,
        serde_json::to_string(&request.genes).unwrap_or_else(|_| "{}".into()),
        if request.diagnostics.is_empty() {
            "(none)"
        } else {
            &request.diagnostics
        }
    )
}

fn parse_mutation(content: &str) -> Result<MutationResponse> {
    let json_text = strip_fences(content);
    let value: serde_json::Value = serde_json::from_str(json_text.trim()).map_err(|e| {
        err(format!(
            "the local model did not return JSON ({e}): {json_text}"
        ))
    })?;
    let law = value
        .get("law")
        .and_then(|v| v.as_str())
        .and_then(LawKind::parse);
    let mut edits = Vec::new();
    if let Some(genes) = value.get("genes").and_then(|v| v.as_object()) {
        for (name, raw) in genes {
            let Some(number) = raw.as_f64() else {
                continue;
            };
            let owned = match_gene_name(name);
            if let Some(static_name) = owned {
                edits.push(GeneEdit {
                    name: static_name,
                    value: number,
                    floor: Floor::Set,
                });
            }
        }
    }
    let note = value
        .get("note")
        .and_then(|v| v.as_str())
        .unwrap_or("local model proposal")
        .to_string();
    Ok(MutationResponse { law, edits, note })
}

fn match_gene_name(name: &str) -> Option<&'static str> {
    crate::genome::BOUNDS
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(known, _)| *known)
}

/// Drop one leading and one trailing markdown fence.
///
/// A language tag on the opening fence (`json`, `rust`, empty) goes with it.
/// The body keeps its leading indent. With no fence, the trimmed text is
/// returned, which is what the mutation parser already relied on.
pub(crate) fn strip_fences(text: &str) -> String {
    let trimmed = text.trim();
    let Some(after) = trimmed.strip_prefix("```") else {
        return trimmed.to_string();
    };
    let body = match after.find('\n') {
        Some(index) if is_fence_tag(&after[..index]) => &after[index + 1..],
        _ => after,
    };
    let body = body.trim_end();
    let body = match body.strip_suffix("```") {
        Some(kept) => kept.trim_end(),
        None => body,
    };
    body.to_string()
}

fn is_fence_tag(tag: &str) -> bool {
    tag.trim()
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    temperature: f64,
    messages: [ChatMessage<'a>; 2],
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatContent,
}

#[derive(Deserialize)]
struct ChatContent {
    content: String,
}

pub(crate) fn chat(
    endpoint: &str,
    model: &str,
    system: &str,
    user: &str,
    timeout: Duration,
) -> Result<String> {
    let url = format!("{endpoint}/chat/completions");
    let body = serde_json::to_string(&ChatRequest {
        model,
        temperature: 0.2,
        messages: [
            ChatMessage {
                role: "system",
                content: system,
            },
            ChatMessage {
                role: "user",
                content: user,
            },
        ],
    })?;
    let response = http::post_json(&url, &body, timeout)?;
    let parsed: ChatResponse = serde_json::from_str(&response)
        .map_err(|e| err(format!("the local model response was not chat JSON ({e})")))?;
    parsed
        .choices
        .into_iter()
        .next()
        .map(|choice| choice.message.content)
        .ok_or_else(|| err("the local model returned no choices"))
}

const SYSTEM_MUTATE: &str = "\
You are the structural mutation operator inside Crab Cage, an offline controller search. \
You propose a control law and numeric genes for a jacketed tank. \
The plant has many measured inputs (noisy delayed temperature, setpoint, ambient, coolant flow, inlet temperature, leak) \
and two actuators (heater duty, coolant valve) plus a one-step temperature prediction. \
Reply with JSON only. Never claim to call a network. Never emit unsafe Rust.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_enables_integral_then_stops() {
        let first = mock_proposal(1, LawKind::Proportional);
        assert_eq!(first.law, Some(LawKind::Pi));
        let again = mock_proposal(2, LawKind::Pi);
        assert!(again.law.is_none());
    }

    #[test]
    fn endpoint_must_be_loopback_http() {
        assert!(normalize_endpoint("http://127.0.0.1:11434/v1").is_ok());
        assert!(normalize_endpoint("http://localhost:8080").is_ok());
        assert!(normalize_endpoint("http://[::1]:8080/v1").is_ok());
        assert!(normalize_endpoint("https://127.0.0.1:11434").is_err());
        assert!(normalize_endpoint("http://api.openai.com/v1").is_err());
        assert!(normalize_endpoint("http://10.0.0.8:11434").is_err());
        assert!(normalize_endpoint("http://8.8.8.8").is_err());
    }

    #[test]
    fn parses_a_fenced_proposal() {
        let text = "```json\n{\"law\":\"pi\",\"genes\":{\"ki\":0.03},\"note\":\"offset\"}\n```";
        let parsed = parse_mutation(text).unwrap();
        assert_eq!(parsed.law, Some(LawKind::Pi));
        assert_eq!(parsed.edits.len(), 1);
        assert_eq!(parsed.edits[0].name, "ki");
    }

    #[test]
    fn strips_a_language_fence_and_keeps_the_region_indent() {
        assert_eq!(
            strip_fences("```rust\n    xs.iter().sum()\n```\n"),
            "    xs.iter().sum()"
        );
        assert_eq!(
            strip_fences("```json\n{\"law\":\"pi\"}\n```"),
            "{\"law\":\"pi\"}"
        );
        assert_eq!(strip_fences("  plain  "), "plain");
    }
}
