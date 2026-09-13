//! Minimal JSON-RPC stdio adapter. The owned supervisor enforces exact live run scope.
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, Write},
    path::Path,
};
fn schema(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn catalog() -> Vec<Value> {
    [
("get_context","Read your Flere workspace, assignment and pending coordination.",schema(json!({}),&[])),
("add_project","Add an existing Git project. Creates or reuses its primary card at the main checkout, including when cwd points into a linked worktree. Starts no shell or agent and preserves focus. All agents have this tool. Additional project cards use worktrees.",schema(json!({"cwd":{"type":"string"},"name":{"type":"string"}}),&["cwd"])),
("list_workspaces","Inspect workspace/run identities and dispatch receipts. Shells and prepared cards are not working agents.",schema(json!({}),&[])),
("prepare_workspace","Create a worker card without a redundant shell or native launch. Supply name/cwd to add a project or create an additional worktree card or name/repository/branch/base for a Git worktree; either accepts optional project. Returns the workspace ID for prepare_worker; inspect list_workspaces after a lost reply before repeating creation. Existing terminals remain intact.",schema(json!({"name":{"type":"string"},"cwd":{"type":"string"},"repository":{"type":"string"},"branch":{"type":"string"},"base":{"type":"string"},"project":{"type":"string"}}),&["name"])),
("update_workspace","update card name/project/pin/status/notes/issue/pr. Read list_workspaces first: expected_epoch must match; status/notes/issue/pr require expected {name,meta} copied exactly from the target. Any supplied expected must still match current metadata; on conflict re-read and reconcile. Legacy name/project/pin-only calls may omit expected. Omitted fields stay unchanged; empty text clears project/notes/issue/pr. No Done/acceptance, archive, identity, focus, terminal or publication changes.",schema(json!({"workspace":{"type":"integer"},"expected_epoch":{"type":"string"},"expected":schema(json!({"name":{"type":"string"},"meta":{"type":"object","description":"Complete, unchanged meta object from this target in list_workspaces."}}),&["name","meta"]),"name":{"type":"string","minLength":1,"maxLength":256},"project":{"type":"string","maxLength":2048},"pinned":{"type":"boolean"},"status":{"type":"string","enum":["todo","in-progress","needs-me","waiting"]},"notes":{"type":"string","maxLength":65536},"issue":{"type":"string","maxLength":2048},"pr":{"type":"string","maxLength":2048}}),&["workspace","expected_epoch"])),
("prepare_worker","save an immutable assignment for a fresh Codex worker in an existing ready workspace. Starts nothing. Capture the actual user request as context, never as an approval credential. Reuse request_id on retry.",schema(json!({"workspace":{"type":"integer"},"request_id":{"type":"string"},"expected_epoch":{"type":"string"},"expected_cwd":{"type":"string"},"assignment":{"type":"string","maxLength":16384},"user_request":{"type":"string","maxLength":4096}}),&["workspace","request_id","expected_epoch","expected_cwd","assignment","user_request"])),
("start_worker","explicitly launch the exact prepared Codex assignment when the conversation authorizes implementation work. This starts a model session that can edit/test within its native permissions. Do not use after a native approval refusal or as a workaround. A repeated ID reports the existing attempt; it never launches a duplicate. Inspect worker_status until actual assignment acknowledgement; launch is not completion.",schema(json!({"dispatch_id":{"type":"string"}}),&["dispatch_id"])),
("worker_status","inspect host/native startup, assignment surfaced/acknowledged and live activity independently; does not start or wake anything.",schema(json!({"dispatch_id":{"type":"string"}}),&["dispatch_id"])),
("ack_assignment","Acknowledge reading and taking up the assignment returned by get_context for this exact native run. Does not accept completed work or grant permissions.",schema(json!({"dispatch_id":{"type":"string"}}),&["dispatch_id"])),
("report_worker_block","record a concrete refusal or blocker against unlaunched preparation. The blocked ID will never launch; do not work around a native refusal. A new attempt requires the actual human clarification and native review.",schema(json!({"dispatch_id":{"type":"string"},"reason":{"type":"string"}}),&["dispatch_id","reason"])),
("cancel_worker","cancel unlaunched preparation and retain its record. Never stops an existing or possibly started worker.",schema(json!({"dispatch_id":{"type":"string"},"reason":{"type":"string"}}),&["dispatch_id","reason"])),
("messaging_activation","Inspect this workspace's exact native hook/queue activation and human-controlled exact-UUID resume instructions. Starts nothing and grants no trust.",schema(json!({}),&[])),
("message_status","Read one message and separate saved, queued, surfaced and acknowledged state. Own recipient reads surface that message; never acknowledges it.",schema(json!({"id":{"type":"string"}}),&["id"])),
("set_focus","Set do-not-disturb for this exact native run (0 clears, at most 1800 seconds). Defers automatic notices and native queue delivery without losing messages.",schema(json!({"seconds":{"type":"integer","minimum":0,"maximum":1800},"reason":{"type":"string","maxLength":1024}}),&["seconds"])),
("deliver_message","Sender or recipient: request a saved message's native delivery to a fresh exact session/run. Respects DND, active work, drafts and native approvals; cannot restart chats, type input or repeat uncertain queue attempts.",schema(json!({"id":{"type":"string"},"session":{"type":"integer"},"run":{"type":"string"}}),&["id","session","run"])),
("inbox","Read/surface messages. Acknowledge only exact IDs already handled.",schema(json!({"ack_ids":{"type":"array","items":{"type":"string"}}}),&[])),
("send_message","Save a durable message. The service checks exact idle native delivery and trusted hooks surface at tool boundaries. Saved/queued never means acknowledged. Inspect message_status for activation or waiting reasons.",schema(json!({"to":{"oneOf":[{"type":"integer"},{"type":"string","enum":["user"]}]},"body":{"type":"string"},"intent":{"type":"string","enum":["quiet","interrupt"]}}),&["to","body"])),
("checkpoint","Save progress and next action; does not finish or accept work.",schema(json!({"body":{"type":"string"}}),&["body"])),
("request_decision","Prepare a concrete human question, recommendation and evidence. Never grants permission.",schema(json!({"question":{"type":"string"},"recommendation":{"type":"string"},"evidence":{"type":"string"}}),&["question","recommendation"])),
("submit_result","Request human review. Does not accept or publish work.",schema(json!({"body":{"type":"string"}}),&["body"])),
("set_status","Set workflow status independently of process state; done never implies acceptance.",schema(json!({"status":{"type":"string","enum":["todo","in-progress","needs-me","waiting"]}}),&["status"])),
("read_terminal","Read bounded recent output of this exact native terminal.",schema(json!({"lines":{"type":"integer","minimum":1,"maximum":200}}),&[])),
("show_workspace","Show a workspace only in response to an explicit user request. Types no input.",schema(json!({"workspace":{"type":"integer"},"user_requested":{"type":"boolean"},"reason":{"type":"string"}}),&["workspace","user_requested","reason"]))
].into_iter().map(|(name,description,input)|json!({"name":name,"description":description,"inputSchema":input})).collect()
}
pub fn serve(state: &Path) -> io::Result<()> {
    let id = std::env::var("FLERE_SESSION").map_err(io::Error::other)?;
    let run = std::env::var("FLERE_RUN").map_err(io::Error::other)?;
    let input = io::stdin();
    let mut reader = io::BufReader::new(input.lock());
    let mut output = io::stdout().lock();
    loop {
        let mut line = Vec::new();
        let n = reader.by_ref().take(131073).read_until(b'\n', &mut line)?;
        if n == 0 {
            return Ok(());
        }
        if line.len() > 131072 {
            return Err(crate::wire::invalid("MCP request exceeds bound"));
        }
        let request: Value = match serde_json::from_slice(&line) {
            Ok(v) => v,
            Err(_) => {
                writeln!(
                    output,
                    "{}",
                    json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"invalid JSON"}})
                )?;
                output.flush()?;
                continue;
            }
        };
        let Some(rid) = request.get("id") else {
            continue;
        };
        let method = request["method"].as_str().unwrap_or_default();
        let result = match method {
            "initialize" => Ok(
                json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"flere","version":env!("CARGO_PKG_VERSION")},"instructions":"Get context first; read inbox at checkpoints; explicitly acknowledge handled IDs. Any agent: use prepare_worker/start_worker/worker_status for authorized implementation assignments; a card or shell alone is not a working agent. Report native approval refusals, never route around them. Native hooks and exact idle queue delivery require native activation; inspect messaging_activation. DND defers attention. Native trust and publication remain human-controlled."}),
            ),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools":catalog()})),
            "tools/call" => {
                let name = request["params"]["name"].as_str().unwrap_or_default();
                let args = request["params"]
                    .get("arguments")
                    .cloned()
                    .unwrap_or(json!({}));
                let op = if name == "get_context" {
                    "context"
                } else {
                    name
                };
                match crate::wire::request(
                    state,
                    &[
                        "agent-operation",
                        &id,
                        &run,
                        op,
                        &crate::wire::hex(&serde_json::to_vec(&args).map_err(io::Error::other)?),
                    ],
                ) {
                    Ok(bytes) => Ok(
                        json!({"content":[{"type":"text","text":String::from_utf8_lossy(&bytes)}]}),
                    ),
                    Err(e) => Ok(
                        json!({"isError":true,"content":[{"type":"text","text":crate::wire::passive(&e.to_string())}]}),
                    ),
                }
            }
            _ => Err(json!({"code":-32601,"message":"unknown method"})),
        };
        let response = match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":rid,"result":result}),
            Err(error) => json!({"jsonrpc":"2.0","id":rid,"error":error}),
        };
        writeln!(output, "{response}")?;
        output.flush()?;
    }
}
use std::io::Read;
