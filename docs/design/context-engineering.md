# Context engineering for Flere coordination

This review connects primary research to Flere's coordination API. Its recommendation is progressive disclosure: return the smallest **sufficient** working context, keep original records durable, and make missing detail easy to retrieve. Smaller responses are an engineering result; better task completion and fewer compactions still require evaluation with actual agents.

## What the evidence supports

| Source | Finding and limits | Application to Flere |
| --- | --- | --- |
| Liu et al., *Lost in the Middle* ([paper, v3](https://arxiv.org/abs/2307.03172v3)) | Multi-document QA and key-value retrieval depend on the position of relevant information. Several tested models perform worse when it is in the middle. These are 2023-era models and constrained tasks, not a measurement of current Codex behavior. | Do not bury current work beneath board inventories, old checkpoints and repeated tool output. Test retrieval of constraints at different positions, not just whether a prompt fits. |
| Hsieh et al., *RULER* ([COLM 2024 paper, v3](https://arxiv.org/abs/2404.06654v3)) | Thirteen tasks across 17 models expose weaknesses that a single needle-in-a-haystack test misses, including multi-hop tracing and aggregation. Effective performance can decline well before a model's advertised context limit. | Test whether agents combine an assignment, a later correction and an unread message correctly. A small JSON response or successful ID lookup alone does not establish useful context. |
| Anthropic, *Effective context engineering for AI agents* ([engineering article](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)) | Recommends just-in-time retrieval, lightweight references, progressive disclosure, structured notes and careful compaction. Warns that retrieval adds latency and aggressive summaries can lose essential details. This is practitioner guidance, not a controlled comparison of Flere designs. | Supply current identity/task and pending-work references up front. Retrieve message bodies and card details only when needed. Keep discovery and detail tools obvious. |
| Zhang et al., *Agentic Context Engineering* ([ICLR 2026 paper, v3](https://arxiv.org/abs/2510.04618v3)) | Identifies brevity bias and loss of detail during repeated whole-context rewriting. Its itemized playbook uses incremental updates and deterministic merging. Results cover specific agent/finance benchmarks and depend on the quality of reflection; the authors explicitly note that some tasks need only concise instructions. | Keep exact decisions, provenance and source records. Update individual task facts rather than repeatedly asking a model to rewrite all history. Do not infer that every agent should receive a growing global playbook. |
| Cemri et al., *Why Do Multi-Agent LLM Systems Fail?* ([NeurIPS 2025 paper, v3](https://arxiv.org/abs/2503.13657v3)) | MAST groups 14 failure modes into system design, inter-agent misalignment and task verification, using 1,642 traces across seven frameworks. Examples include repetition, ignored input, lost history and premature termination. Annotation and framework/task selection limit generalization; the taxonomy is not proof of any particular local cause. | Evaluate lost messages, duplicate work, ignored corrections and unsupported completion claims. Delivery, reading, handling and verified outcomes need separate evidence. |
| Kim et al., *Towards a Science of Scaling Agent Systems* ([paper, v3](https://arxiv.org/abs/2512.08296v3)) | Across 260 configurations, six benchmarks and five architectures, collaboration helps some decomposable tasks and harms some sequential tasks. Coordination costs and error propagation matter. Its predictive model has modest cross-validated fit; coding/CLI subsets contain only 20 instances per benchmark and some comparisons have wide intervals. | Delegate separable work and verify integration. Avoid all-to-all transcript sharing and fixed agent-count recipes. Benchmark a capable single-agent baseline at comparable total cost. |
| Anthropic, *How we built our multi-agent research system* ([engineering article](https://www.anthropic.com/engineering/multi-agent-research-system)) | Reports strong internal research-evaluation gains, alongside substantial token cost and fewer opportunities to parallelize coding. The often-cited 15× token figure compares its multi-agent workload with chats, not an equivalent single-agent coding workflow. | Workers can return compact evidence and artifacts while keeping exploration local. Do not copy the article's hierarchy or cost claims as universal rules. |
| OpenAI, *Conversation state* ([product documentation](https://developers.openai.com/api/docs/guides/conversation-state)) | Describes context limits and links to provider compaction mechanisms. This specifies API behavior, not evidence that a particular summary improves task success. | Reduce future Flere tool output. Native conversation compaction and already-injected history remain the harness/provider's responsibility. |

The useful synthesis is **small working views over durable, retrievable evidence**. “Always summarize more” and “always use more agents” are both unsupported shortcuts.

## Principles to apply

### 1. Separate durable state from working context

Messages, decisions, assignments, dispatch identities and original evidence are durable records. A tool response is a task-specific view over those records. Acknowledging a message changes its pending state; it does not delete the original record or retract content already sent to a model.

The default view should answer: Which workspace/run am I in? What assignment and constraints apply? What is the latest progress checkpoint? Is there pending mail? Most agents do not need every card's notes or a backlog of completed checkpoints to answer those questions.

The current implementation retains the full assignment and decision records, returns one latest checkpoint, and lists pending message summaries. This deliberately does **not** promise a universal byte cap for `get_context`: a large assignment, checkpoint or decision history can still be large. Further structured decision retrieval should be evaluated before hiding potentially binding answers.

### 2. Use deterministic projections before model-generated summaries

Removing unrelated card notes from an inventory is deterministic and reversible. Replacing a decision with a shorter paraphrase can change its meaning. Prefer IDs, exact fields, counts and explicit detail reads for transport optimization.

Keep the original body and source provenance accessible. Do not turn an agent-written summary of a user request into an approval credential. A compact view should state when it is incomplete, and metadata writes must still compare against the complete current record.

### 3. Make progressive disclosure navigable

An agent should not need to guess that more data exists. Return a pending count, a continuation cursor and a concise instruction identifying the detail operation. Use stable IDs rather than offsets into a changing unread list.

Inbox pages retain whole messages. The count limit and serialized-record byte target work together; a single oversized record is returned whole so it cannot permanently block the queue. A cursor is scoped to the recipient conversation. Acknowledging earlier pages must not shift subsequent records out of reach. Returning to the first page rediscovers still-pending work.

There is a cost: an extra read adds latency and tool framing. Tiny messages can be cheaper inline. Start with predictable, inspectable paging and measure total workflow cost before adding adaptive inline thresholds, search indexes or embeddings.

### 4. Distinguish notification, reading, handling and outcome

A notice or summary says that work exists. A full body read makes that content available to the recipient. An explicit acknowledgement records that the recipient says it handled the ID. None of those alone proves that code was fixed, a test passed or a remote action completed.

Keep queue submission, native surfacing and acknowledgement distinct. ACK-only calls should return a receipt instead of implicitly loading another page. Retries must preserve request IDs and deduplication. A stopped or different conversation must never silently become the recipient.

Flere already has transport-specific surfacing receipts: a native hook/MCP notice can record notice delivery without a body read. Compact context does not create a new such receipt. Consumers must still inspect the body and exact acknowledgement rather than treating any transport timestamp as completion.

### 5. Make communication sparse and relevant

Send a message when a dependency, decision, blocker, result or ownership handoff changes. A useful handoff includes scope, current revision/artifact, relevant constraints, evidence, remaining work and next owner. It should reference bulky logs instead of copying them.

Target exact recipients for task handoffs. Channels/subscriptions could help discovery later, but indiscriminate broadcasts would multiply the problem: one long status update becomes many agents' context. Any future subscription design needs explicit membership, bounded catch-up, deduplication and a way to leave. It should not be the first fix for oversized reads.

### 6. Match parallelism to the dependency graph

Independent investigation or a separate review can benefit from isolated context. Tightly coupled edits, sequential migrations and shared mutable state may cost more to coordinate than they save.

Assign a concrete output and acceptance condition, then verify the combined result. A temporary coordinating responsibility does not require a privileged agent role. Flere agents remain peers; pinned cards are presentation, not authority. Native acceptance and approval boundaries do not change because another agent relayed a message.

### 7. Put mechanism in Flere and judgment in skills

| Flere responsibilities | Skill/workflow responsibilities |
| --- | --- |
| Exact identities, conversation routing, durable records and retry receipts | Task decomposition and deciding whether delegation is worthwhile |
| Compact defaults, complete detail reads, pagination and truthful pending counts | Selecting evidence and writing useful short checkpoints/handoffs |
| Atomic acknowledgements, stale-metadata guards and save-failure rollback | Acting on messages within the assignment and acknowledging handled IDs |
| Read efficiency, storage accounting and lifecycle-safe retention mechanisms | Verifying outcomes, respecting scope and deciding when a human must participate |

Do not encode project-specific “leader” privileges, review rituals or approval-transfer rules into transport. Conversely, a skill cannot compensate reliably for silently dropped pages or an inventory that always injects all notes.

### 8. Treat storage retention as a separate design

Whitespace-free JSON reduces storage overhead without dropping records or changing the schema. It is not garbage collection. Suppressing unchanged inbox writes reduces work but does not stop long-term record growth.

A later archival/pruning design should preserve pending messages, unanswered decisions, active dispatches, exact retry identities and evidence needed by ongoing work. It needs explicit retention semantics, pagination/cursor behavior across archival, deduplication tombstones, crash recovery, export and storage-pressure reporting. Avoid silently deleting old records solely to hit a context-size target. This change introduces no destructive pruning or live migration.

## Implementation and evaluation

The first implementation covers compact context/card reads, targeted full metadata, paginated inbox bodies, slim mutation receipts, shorter MCP initialization guidance, unchanged-read fast paths and compact persisted JSON. Full human context stays available. See the [coordination API guide](../guides/coordination.md#bounded-agent-context-and-inbox-pages).

Automated proof uses real supervisors, Unix sockets and the stdio MCP adapter with disposable homes and harmless native-process stand-ins. Tests cover inventory paging, oversized messages, exact body preservation, acknowledgement receipts, failed-save rollback, stale metadata rejection, retained decisions/checkpoints and no rewrite on unchanged reads. Existing chat tests cover conversation provenance, request deduplication, live refresh and cold restart. These fixtures establish protocol behavior, not model comprehension or native trust acceptance.

For a subsequent agent evaluation, compare the prior behavior, the compact behavior and a capable single-agent baseline where relevant. Use the same task set, models and overall resource limits; report differences in consumed tokens rather than assuming a fixed tokens-per-byte ratio.

| Measure | Why it matters |
| --- | --- |
| Serialized bytes and actual tokenizer counts per operation and completed task | Detect savings that are offset by extra calls or repeated detail retrieval |
| Tool calls, p50/p95 latency and persistence writes | Detect progressive-disclosure or storage overhead |
| Correct task completion and constraint recall | Ensure compactness preserves useful behavior |
| Missed messages, duplicate actions, wrong recipients and stale updates | Verify coordination correctness under paging and retries |
| Compactions and human “check messages/continue” interventions | Measure the user-visible outcome |
| Resumption after compaction, disconnect and restart | Exercise continuity rather than only a fresh happy path |

Include long notes, interleaved acknowledgements/new arrivals, multiple conversations in one card, large escaped/Unicode bodies, changed decisions and deliberately stale evidence. Classify failures with MAST where useful, but confirm causes against traces. Do not claim that response-size reductions alone fix input lag, approval transfer or all premature compaction.
