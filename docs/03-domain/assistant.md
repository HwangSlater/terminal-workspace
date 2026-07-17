# AI Assistant Specification

This document details the Assistant Bounded Context, which orchestrates local LLM integrations, conversational streams, and contextual tools.

---

## 1. Domain Entities & Value Objects

```rust
pub struct Conversation {
    pub id: SessionId,
    pub messages: Vec<Message>,
    pub created_at: EpochMs,
}

pub struct Message {
    pub id: MessageId,
    pub role: AuthorRole, // User, Assistant, System, Tool
    pub content: String,
    pub token_estimate: u32,
}

pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters_schema_json: String,
}
```

---

## 2. Contextual Tool Execution (Agent Flow)

To answer developer queries accurately, the Assistant has access to local Workspace tools:

```text
    User: "Who approved PR 42?"
              │
              ▼
      [System Prompt] ──(Context: Active Workspace State)
              │
              ▼
        [LLM Model] ──(Requires Tool)──> [Tool: QueryGitHubPr(42)]
              │                                      │
              ▼                                      ▼
      [Tool Response] <────────────────────────[Execute Tool]
              │
              ▼
      [LLM Formulation]
              │
              ▼
  Assistant: "@bob approved it 2h ago"
```

### Available Tools:
1. `read_active_notifications()`: Queries the Notification Context cache.
2. `query_github_pr(repo, id)`: Fetches PR status from the GitHub Adapter.
3. `search_workspace_logs(query)`: Greps the local application logs database.
4. `get_workspace_state()`: Reads TUI window states and focused pane metadata.
5. `dispatch_command(command_str)`: Dispatches a command to the `CommandRegistry` on behalf of the AI.

---

## 3. Vector & Memory Storage
- **Memory**: The local conversation state is cached inside SQLite (`chat_history` table).
- **Embeddings Context**: (Optional) Utilizes a lightweight local vector library (e.g., hnsw) to store chunked documentation or file paths for fast retrieval during `/explain` commands.
