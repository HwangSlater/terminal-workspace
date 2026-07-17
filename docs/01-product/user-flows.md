# User Flows Specification

This document maps the stateful transition paths of user interactions inside the Terminal Workspace.

---

## 1. Startup & Authentication Flow

On executing `terminal-workspace` in the console:

```mermaid
graph TD
    A[Launch Terminal App] --> B{Local SQLite DB Exists?}
    B -- No --> C[Execute SQL Migrations]
    B -- Yes --> D[Run Database Integrity Check]
    C --> E[Initialize SecretProvider Chain]
    D --> E
    E --> F{Are Tokens Valid?}
    F -- No --> G[Prompt Interactive Console Setup / Keychain Save]
    F -- Yes --> H[Start Background Integrations]
    H --> I[Initialize UI Docking States]
    I --> J[Render Dashboard]
```

---

## 2. Notification Reception & Reply Flow

When a Slack DM or Github Review Request occurs:

```mermaid
graph TD
    A[Integration Adapter Receives Msg] --> B[Convert to Event Entity]
    B --> C[Post to EventBus]
    C --> D[Notification Pipeline Rule Engine]
    D -->|Match Rules| E[Assign Priority & Deduplicate]
    E --> F[Cache to SQLite Database]
    F --> G[Mutate ReadModel Projection]
    G --> H[Trigger UI Rerender]
    H --> I[User presses F2: Focus Notification View]
    I --> J[User inputs ':' for Cmd Mode]
    J --> K[User types '/reply <id> I will check now']
    K --> L[Publish SendMessageCommand]
    L --> M[Slack Integration Adapter Sends HTTP Request]
```

---

## 3. Graceful Termination Flow

When the user exits via `Ctrl + Q`:

```mermaid
graph TD
    A[Ctrl + Q Captured] --> B[Broadcast ShutdownCommand]
    B --> C[PluginManager triggers shutdown on WASM instances]
    C --> D[Integrations close WebSockets / EventStreams]
    D --> E[StorageService flushes memory cache to SQLite]
    E --> F[Reset Console Screen Buffer via Crossterm]
    F --> G[Exit Process 0]
```
