# Terminal Workspace

터미널을 벗어나지 않고 Slack, GitHub, Gmail, Google Calendar, Jira, CI/CD를 한곳에서 확인하고 다루는 터미널 우선 개발자 워크스페이스입니다.

Local First. Zero Configuration. Windows·macOS·Linux 어느 OS도 2등 시민 취급하지 않는 크로스 플랫폼 — 자세한 내용은 [`docs/06-development/platform-support.md`](docs/06-development/platform-support.md) 참고.

---

## 시작하기

**1. Rust 설치** (아직 없으시다면): <https://rustup.rs> — 이것만 있으면 됩니다. C 컴파일러도, 별도 데이터베이스 서버도, 추가 툴체인도 필요 없습니다 (저장소가 순수 Rust `redb`라서 그렇습니다 — [ADR-0014](docs/06-development/decisions/0014-storage-engine-reconsideration.md) 참고).

**2. 실행:**

```sh
cargo run -p app
```

설정 파일을 손으로 작성할 필요가 없습니다 — 처음 실행하면 `config.toml`과 로컬 데이터베이스가 자동으로 만들어집니다 ([`docs/05-operations/configuration.md`](docs/05-operations/configuration.md) §4 참고).

선택: `scripts/setup.ps1`(Windows) / `scripts/setup.sh`(Linux/macOS)는 한 번에 환경을 점검해주는 스크립트입니다 (`rustup` 존재 여부 확인 후 `cargo check --workspace` 실행) — 본격적으로 시작하기 전에 통과 여부를 명확히 확인하고 싶으실 때 사용하세요.

### Windows에서 실제로 빌드/실행하려면

`redb`(순수 Rust 저장소) 덕분에 C 컴파일러 자체는 필요 없지만, Rust가 실행 파일을 링크하려면 최소한의 링커는 여전히 필요합니다. Windows에서 아래 증상을 만나면:

```
error: error calling dlltool 'dlltool.exe': program not found
```

GCC 기반 MinGW-w64를 설치하면 해결됩니다 (`winget install BrechtSanders.WinLibs.POSIX.UCRT` — 설치 프로그램 없이 압축 해제만 하는 방식이라 가볍고 빠릅니다). LLVM 기반 MinGW은 Rust가 기대하는 `libgcc`/`libgcc_eh`가 없어 링크가 실패하니 피하세요. 설치 후 해당 `mingw64\bin` 폴더를 PATH에 추가하고 새 터미널에서 다시 시도하세요.

---

## 사용법

앱을 실행하면(`cargo run -p app`) 터미널 전체 화면을 쓰는 대화형 UI가 뜹니다. 키보드로만 조작합니다 (Vim에서 영감을 받은 모달 입력 방식 — 자세한 설계는 [`docs/02-architecture/keyboard.md`](docs/02-architecture/keyboard.md) 참고):

| 키 | 동작 |
| :--- | :--- |
| `Tab` / `Shift+Tab` | 패널 포커스를 순서대로 이동 |
| `Ctrl+1` ~ `Ctrl+4` | 팀 / 알림 / 캘린더 / 로그 패널로 바로 이동 |
| `j`/`k`, `↑`/`↓` | 포커스된 패널 안에서 위아래로 선택 이동 |
| `:` | 명령줄 입력 모드로 전환 |
| `?` | 도움말 팝업 열기 |
| `Esc` | 명령줄/도움말 닫고 Normal 모드로 복귀 |
| `Ctrl+S` | Slack 연결 설정 |
| `Ctrl+Q` | 종료 |

Slack을 연동하면 팀·알림 패널에 실제 메시지/프레즌스가 표시됩니다. 캘린더·CI/CD·AI 어시스턴트 패널은 아직 준비 중입니다 — 왜 이렇게 범위를 나눴는지는 [`step5.md`](step5.md)를 참고하세요.

### Slack 연동

1. Slack 워크스페이스에 App을 하나 만들고(Slack "Create New App"), Bot Token 스코프로 `channels:history`, `channels:read`, `users:read`, `chat:write`를 추가한 뒤 워크스페이스에 설치해 Bot Token(`xoxb-...`)을 발급받으세요.
2. 앱을 실행하고 `Ctrl+S`를 눌러 Bot Token을 붙여넣은 뒤 Enter — 저장과 동시에 바로 연결을 시도합니다. 토큰은 OS 키체인(Windows 자격 증명 관리자 / macOS 키체인 / Linux Secret Service)에 영구 저장되고, 없으면 로컬 암호화 파일로 대체 저장됩니다. 설정 파일(`config.toml`)에는 절대 들어가지 않습니다.
3. 메시지를 받아올 채널과 프레즌스를 볼 팀원은 아직 `config.toml`의 `[integrations.slack]`에서 `channel_ids`/`watched_user_ids`로 직접 지정해야 합니다 (UI에서 고르는 기능은 다음 단계).

자세한 내용은 [`docs/04-extensions/integrations/slack.md`](docs/04-extensions/integrations/slack.md)와 [`step7.md`](step7.md)를 참고하세요.

---

## 진행 현황

이 프로젝트는 아키텍처 우선(Architecture First) 방식으로 개발 중입니다. Phase 2(핵심 인프라: Event Bus, Registry, Config, Secrets, Logging), Phase 3(Storage + CQRS 쓰기 경로), Phase 4(cargo-dist 릴리스 패키징), Phase 5(대화형 TUI 셸), Phase 6(첫 실제 연동인 Slack), Phase 7(앱 안에서 바로 Slack 연결 설정 + OS 키체인 영구 저장)까지 구현되어 있습니다 — 각 단계가 무엇을 다루고 왜 그렇게 했는지는 [`step2.md`](step2.md), [`step3.md`](step3.md), [`step4.md`](step4.md), [`step5.md`](step5.md), [`step6.md`](step6.md), [`step7.md`](step7.md)를 참고하세요.

## 문서

전체 아키텍처, 설계 결정, 명세는 [`docs/`](docs/README.md)에 있습니다 — "어떻게 실행하는지"를 넘어서는 내용은 여기서부터 보시면 됩니다.

## 개발

- `cargo check --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo fmt --check` / `cargo test --workspace`
- 코드 스타일, 기능 변경 절차, 이 코드베이스가 따르는 Architecture Freeze v1 규칙은 [`docs/06-development/development.md`](docs/06-development/development.md)를 참고하세요.
