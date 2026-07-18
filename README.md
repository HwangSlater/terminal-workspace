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

선택: `scripts/setup.ps1`(Windows) / `scripts/setup.sh`(Linux/macOS)는 한 번에 환경을 점검해주는 스크립트입니다 — `rustup` 존재 여부 확인, (Linux/macOS는) 빠진 게 있으면 정확한 설치 명령어까지 알려준 뒤(아무것도 대신 설치하지는 않습니다), `cargo check --workspace` 실행까지 한 번에 해줍니다. 본격적으로 시작하기 전에 통과 여부를 명확히 확인하고 싶으실 때 사용하세요.

### Windows에서 실제로 빌드/실행하려면

`redb`(순수 Rust 저장소) 덕분에 C 컴파일러 자체는 필요 없지만, Rust가 실행 파일을 링크하려면 최소한의 링커는 여전히 필요합니다. Windows에서 아래 증상을 만나면:

```
error: error calling dlltool 'dlltool.exe': program not found
```

GCC 기반 MinGW-w64를 설치하면 해결됩니다 (`winget install BrechtSanders.WinLibs.POSIX.UCRT` — 설치 프로그램 없이 압축 해제만 하는 방식이라 가볍고 빠릅니다). LLVM 기반 MinGW은 Rust가 기대하는 `libgcc`/`libgcc_eh`가 없어 링크가 실패하니 피하세요. 설치 후 해당 `mingw64\bin` 폴더를 PATH에 추가하고 새 터미널에서 다시 시도하세요.

이 MinGW 설치는 `crates/plugin-host`(Phase 14, 플러그인 런타임)를 빌드할 때도 그대로 씁니다 — `wasmtime`이 진짜 C 컴파일러를 요구하기 때문입니다(ADR-0017). 플러그인 작업을 안 하신다면 `cargo check -p <크레이트>`로 개별 크레이트만 빌드해서 이 요구사항을 피할 수 있습니다.

### macOS

1. Rust 설치: <https://rustup.rs>
2. Xcode Command Line Tools가 필요합니다 (Rust가 아니라 macOS에서 뭘 빌드하든 필요한 최소 링커이자 실제 C 컴파일러 clang) — 이미 있는지 `xcode-select -p`로 확인, 없으면 `xcode-select --install`. `crates/plugin-host`(Phase 14, `wasmtime`)도 이걸로 충분합니다 — macOS는 별도로 더 설치할 게 없습니다(ADR-0017).
3. `git clone` 후 `cargo run -p app` — 그 외 추가 설치는 없습니다. Slack 토큰은 macOS 키체인(Keychain Services)에 자동으로 저장됩니다.

### Linux

1. Rust 설치: <https://rustup.rs>
2. 이 프로젝트는 HTTP 통신(`reqwest`)에 OS 기본 TLS를 씁니다 — Windows는 자체 내장 TLS, macOS는 Security.framework를 쓰지만, **Linux는 시스템 OpenSSL이 필요**합니다:
   - Debian/Ubuntu: `sudo apt install build-essential libssl-dev pkg-config`
   - Fedora/RHEL: `sudo dnf install gcc openssl-devel pkgconf-pkg-config`
   - Arch: `sudo pacman -S base-devel openssl pkgconf`

   (OpenSSL 소스를 직접 컴파일하는 게 아니라 시스템에 이미 있는 라이브러리를 찾아 연결하는 것뿐이라, 위 패키지들은 보통 데스크톱 배포판엔 이미 있습니다.)

   위 `build-essential`/`gcc` 패키지는 원래 이 프로젝트에서 "링커만 있으면 됨, 진짜 컴파일러는 불필요"였는데, Phase 14부터는 `crates/plugin-host`(`wasmtime`) 때문에 진짜 C 컴파일러가 필요해졌습니다(ADR-0017) — 위 명령어를 그대로 쓰면 되지만, 이전엔 "어차피 있는 패키지라 곁다리로 충분"했던 것이 이제는 실제 요구사항이라는 점만 다릅니다.
3. `git clone` 후 `cargo run -p app`.
4. Slack 토큰 저장: 데스크톱 환경이면(GNOME/KDE 등) DBus Secret Service(gnome-keyring/kwallet)에 자동 저장됩니다 — **빌드 시점에 `libdbus` 같은 걸 따로 설치할 필요는 없습니다** (순수 Rust DBus 클라이언트를 씁니다). 헤드리스/서버 환경처럼 Secret Service가 아예 없는 경우엔 자동으로 로컬 암호화 파일(`~/.config/terminal-workspace/secrets.enc`)로 대체 저장되니 별도 조치가 필요 없습니다.

---

## 사용법

앱을 실행하면(`cargo run -p app`) 터미널 전체 화면을 쓰는 대화형 UI가 뜹니다. 키보드로만 조작합니다 (Vim에서 영감을 받은 모달 입력 방식 — 자세한 설계는 [`docs/02-architecture/keyboard.md`](docs/02-architecture/keyboard.md) 참고):

| 키 | 동작 |
| :--- | :--- |
| `Tab` / `Shift+Tab` | 패널 포커스를 순서대로 이동 (팀 → 알림 → 캘린더) |
| `Ctrl+1` ~ `Ctrl+3` | 팀 / 알림 / 캘린더 패널로 바로 이동 |
| `Ctrl+4` | 로그 보기 — 최근 기록 전체를 보여주는 오버레이를 바로 엶 |
| `j`/`k`, `↑`/`↓` | 포커스된 패널 안에서 위아래로 선택 이동 |
| `:` | 명령줄 입력 모드로 전환 |
| `?` | 도움말 팝업 열기 |
| `Esc` | 명령줄/도움말/오버레이 닫고 Normal 모드로 복귀 |
| `Ctrl+S` | Slack 연결 설정 |
| `Ctrl+P` | Slack 채널/사용자 선택 |
| `Ctrl+G` | GitHub 연결 설정 |
| `Ctrl+R` | GitHub 저장소 선택 |
| `Ctrl+L` | Calendar 연결 설정 |
| `Ctrl+Q` | 종료 |

Slack/GitHub/Calendar를 연동하면 팀·알림 패널에 실제 메시지/프레즌스/PR/일정 알림이 표시됩니다. CI/CD·AI 어시스턴트 패널은 아직 준비 중입니다 — 왜 이렇게 범위를 나눴는지는 [`step5.md`](docs/07-implementation-log/step5.md)를 참고하세요.

### Slack 연동

1. Slack 워크스페이스에 App을 하나 만들고(Slack "Create New App"), Bot Token 스코프로 `channels:history`, `channels:read`, `users:read`, `chat:write`를 추가한 뒤 워크스페이스에 설치해 Bot Token(`xoxb-...`)을 발급받으세요.
2. 앱을 실행하고 `Ctrl+S`를 눌러 Bot Token을 붙여넣은 뒤 Enter — 저장과 동시에 바로 연결을 시도합니다. 토큰은 OS 키체인(Windows 자격 증명 관리자 / macOS 키체인 / Linux Secret Service)에 영구 저장되고, 없으면 로컬 암호화 파일로 대체 저장됩니다. 설정 파일(`config.toml`)에는 절대 들어가지 않습니다.
3. 메시지를 받을 채널에 봇을 초대하세요 (`/invite @봇이름`) — 봇이 없는 채널은 애초에 메시지를 못 읽습니다.
4. `Ctrl+P`를 눌러 채널/팀원 목록을 불러오고, `j`/`k`로 이동, `Space`로 선택, `Enter`로 저장하세요 — `config.toml`에 바로 반영되고 폴링도 재시작 없이 바로 적용됩니다.
5. 명령줄(`:`)에서 바로 메시지를 보내거나 상태를 바꿀 수 있습니다 (`Ctrl+P`로 불러온 채널 이름 기준):
   - `/send #채널이름 메시지` — Slack 메시지 전송
   - `/away`, `/active`, `/offline`, `/meeting`, `/lunch [메시지]` — 내 상태 변경
   - 명령어나 `#채널이름` 입력 중 `Tab`을 누르면 자동완성됩니다 (셸처럼 연속으로 누르면 다음 후보로 순환).

헤더에 Slack 연결 상태(연결됨/재연결 중/연결 안 됨 등)가 실시간으로 표시됩니다 — 폴링이 백그라운드에서 끊기거나 복구돼도 키를 누르지 않아도 바로 반영됩니다.

자세한 내용은 [`docs/04-extensions/integrations/slack.md`](docs/04-extensions/integrations/slack.md), [`step7.md`](docs/07-implementation-log/step7.md), [`step8.md`](docs/07-implementation-log/step8.md)를 참고하세요.

### GitHub 연동

1. GitHub → Settings → Developer settings → Personal access tokens에서 `repo` 스코프로 Classic PAT(`ghp_...`)를 발급받으세요.
2. 앱을 실행하고 `Ctrl+G`를 눌러 토큰을 붙여넣은 뒤 Enter — 저장과 동시에 바로 연결을 시도합니다. 토큰은 Slack과 동일하게 OS 키체인(또는 암호화 파일 폴백)에 영구 저장되고, `config.toml`에는 절대 들어가지 않습니다.
3. `Ctrl+R`을 눌러 접근 가능한 저장소 목록을 불러오고, `j`/`k`로 이동, `Space`로 선택, `Enter`로 저장하세요 — `config.toml`에 바로 반영되고 폴링도 재시작 없이 바로 적용됩니다.
4. 선택한 저장소의 열린 PR이 알림 패널에 표시됩니다 (새로 열린 PR만 — 수정/닫힘은 아직 추적하지 않습니다).

자세한 내용은 [`docs/04-extensions/integrations/github.md`](docs/04-extensions/integrations/github.md), [`step10.md`](docs/07-implementation-log/step10.md)를 참고하세요.

### Calendar 연동

1. Google Calendar → 설정 → 연동하려는 캘린더 선택 → "캘린더 통합" → **비공개 iCal 형식 주소**를 복사하세요. OAuth 앱 등록이나 로그인 절차가 필요 없습니다 — 이 주소 자체가 토큰입니다.
2. 앱을 실행하고 `Ctrl+L`을 눌러 주소를 붙여넣은 뒤 Enter — 저장과 동시에 바로 연결을 시도합니다. Slack/GitHub과 동일하게 OS 키체인(또는 암호화 파일 폴백)에 영구 저장되고, `config.toml`에는 절대 들어가지 않습니다.
3. 연결하면 앞으로 24시간(`lookahead_hours`로 조절 가능) 이내에 시작하는 일정이 알림 패널에 표시됩니다 — 매일/매주 반복되는 일정(스탠드업 등)도 정확히 인식합니다.
4. 캘린더 목록을 불러오는 피커는 없습니다 — 이 인증 방식 자체에 "내 캘린더 목록 조회" API가 없어서, 연결 하나당 캘린더 하나만 지원합니다.

자세한 내용은 [`docs/04-extensions/integrations/calendar.md`](docs/04-extensions/integrations/calendar.md), [`step12.md`](docs/07-implementation-log/step12.md)를 참고하세요.

### 플러그인 (Phase 14, 실험적)

WebAssembly Component Model(WIT) 기반 샌드박스 플러그인 런타임입니다 — 기본적으로 꺼져 있고, 커맨드/UI 등록 같은 확장 지점은 아직 없이 "로드 → 초기화 → 이벤트 전달 → 종료"라는 생명주기와 자원 제한(CPU/메모리)만 증명하는 단계입니다.

1. `config.toml`의 `[plugins]`에서 `enabled = true`로 켜고, `directory`(플러그인 `.wasm` 파일이 있는 폴더)와 `allowed_list`(로드를 허용할 플러그인 id 목록)를 채우세요 — 둘 다 명시적으로 지정해야 실제로 로드됩니다.
2. 플러그인 작성자는 [`cargo-component`](https://github.com/bytecodealliance/cargo-component)(`cargo install cargo-component`)로 빌드합니다 — `examples/plugins/hello`가 최소 예제입니다 (`initialize`/`on-event`/`shutdown` 세 개만 구현, host 쪽에는 `log`/`publish-event` 두 함수만 노출).
3. 매 이벤트 처리마다 CPU 100만 명령어(fuel), 인스턴스당 메모리 64MB로 제한됩니다 — 초과하면 해당 플러그인만 트랩되어 내려가고(`Suspended`), 워크스페이스 전체는 영향받지 않습니다. `examples/plugins/looper`(무한 루프)와 `examples/plugins/memhog`(과다 할당)가 이 두 제한이 실제로 걸리는지 증명하는 테스트용 예제입니다.
4. **컨트리뷰터 한정 요구사항**: 플러그인 런타임(`wasmtime`)은 진짜 C 컴파일러가 필요합니다 — 위 "Windows에서 실제로 빌드/실행하려면"/macOS/Linux 절 참고(ADR-0017). 프리빌드 릴리스 바이너리를 쓰는 최종 사용자는 전혀 영향받지 않습니다.

자세한 내용은 [`docs/04-extensions/plugin-lifecycle.md`](docs/04-extensions/plugin-lifecycle.md), [`docs/06-development/decisions/0017-plugin-runtime-c-compiler-exception.md`](docs/06-development/decisions/0017-plugin-runtime-c-compiler-exception.md), [`step14.md`](docs/07-implementation-log/step14.md)를 참고하세요.

### 데몬 모드 & 로컬 CLI 소켓 IPC (Phase 15)

앱을 실행하면 그 인스턴스 자체가 "데몬"입니다 — 별도 백그라운드 프로세스나 서비스 설치 없이, 이미 열려 있는 워크스페이스에 다른 터미널에서 짧은 명령으로 바로 접근할 수 있습니다 (예: Vim에서 `:!termws slack-send "@bob" "패치 올렸습니다"`).

```sh
# 이미 실행 중인 인스턴스가 있어야 합니다 (cargo run -p app 로 열어둔 창)
cargo run -p app -- slack-send '#general' 안녕하세요
cargo run -p app -- set-presence away 자리 비움
cargo run -p app -- status
```

- `slack-send <채널> <텍스트>`, `set-presence <active|away|offline|meeting|lunch> [텍스트]`, `status`(연결 상태 + 안 읽은 알림 수) 세 가지만 지원합니다 — 토큰 입력이나 목록 선택이 필요한 것(연동 설정, 채널/저장소 피커)은 TUI 오버레이가 이미 더 잘 처리하므로 제외했습니다.
- 실행 중인 인스턴스가 없으면 조용히 실패하지 않고 명확한 오류를 출력합니다.
- Linux/macOS는 Unix Domain Socket, Windows는 Named Pipe를 사용합니다 (`interprocess` 크레이트) — 인증은 OS 파일 권한에 의존합니다(로컬 단일 사용자 전제).

자세한 내용은 [`docs/01-product/user-flows.md`](docs/01-product/user-flows.md) §3, [`step15.md`](docs/07-implementation-log/step15.md)를 참고하세요.

### Pomodoro 타이머 (Phase 18)

명령줄에서 `/pomodoro start [작업분] [휴식분]`(기본 25/5분), `/pomodoro pause`(일시정지/재개 토글), `/pomodoro reset`으로 조작합니다. 실행 중에는 헤더에 남은 시간이 실시간으로 표시되고(`🍅 24:35 (Work)`), 작업/휴식 세션이 끝나면 터미널 벨이 울리고 자동으로 다음 모드로 전환됩니다.

자세한 내용은 [`step18.md`](docs/07-implementation-log/step18.md)를 참고하세요.

### 로그 보기 (Phase 17, Phase 19/20에서 오버레이로 재설계)

`Ctrl+4`를 누르면 앱 자체의 로그 기록(최근 200줄, 비밀값은 자동으로 가려짐)을 큰 오버레이로 볼 수 있습니다. `ERROR`/`WARN` 줄은 색으로 구분되어 눈에 띕니다. 처음엔 화면 하단에 항상 떠 있는 1줄짜리 패널이었는데, 실제로는 최근 한 줄만 보여서 쓸모가 거의 없다는 게 확인되어(80칸 최소 터미널에서 실측) 지금의 온디맨드 오버레이 방식으로 바뀌었습니다.

자세한 내용은 [`step17.md`](docs/07-implementation-log/step17.md), [`step19.md`](docs/07-implementation-log/step19.md)를 참고하세요.

### 알림을 놓치지 않으려면 (Phase 21, Phase 22)

Slack DM, GitHub PR 리뷰 요청, Calendar 리마인드, Pomodoro 세션 종료 — 이 4가지는 다른 터미널이나 다른 앱을 보고 있어도 알아챌 수 있도록 두 가지 채널로 알려줍니다:

- **데스크톱(OS) 토스트 알림**: Windows/macOS/Linux 어디서든 화면에 시스템 팝업이 뜹니다 (Windows에서 실제 동작 확인됨; Linux/macOS는 다음 실제 CI 실행에서 확인 예정).
- **터미널 탭/창 제목**: 안 읽은 알림이 있으면 탭 제목이 `Terminal Workspace (3)`처럼 바뀝니다 — 다른 탭에서 작업 중이어도 탭 바에서 바로 보입니다.

둘 다 별도 설정 없이 항상 켜져 있고, 실패해도(예: 알림 데몬이 없는 헤드리스 Linux) 앱 자체는 영향받지 않습니다. `Ctrl+Q`로 종료하면 탭 제목은 원래대로 돌아갑니다.

자세한 내용은 [`step21.md`](docs/07-implementation-log/step21.md), [`step22.md`](docs/07-implementation-log/step22.md)를 참고하세요.

---

## 진행 현황

이 프로젝트는 아키텍처 우선(Architecture First) 방식으로 개발 중입니다. Phase 2(핵심 인프라: Event Bus, Registry, Config, Secrets, Logging), Phase 3(Storage + CQRS 쓰기 경로), Phase 4(cargo-dist 릴리스 패키징), Phase 5(대화형 TUI 셸), Phase 6(첫 실제 연동인 Slack), Phase 7(앱 안에서 바로 Slack 연결 설정 + OS 키체인 영구 저장), Phase 8(채널/사용자 UI 피커), Phase 9(명령줄 `/send`·상태 변경 + 실시간 연결상태 표시), Phase 10(두 번째 연동인 GitHub — 폴링, 연결 설정, 저장소 피커까지 한 단계에 구현), Phase 11(연동 2개로 반복되던 패턴을 `Command::Connect`/`ApplySelection` 등으로 일반화 — Calendar 붙이기 전에 정리), Phase 12(세 번째 연동인 Calendar — OAuth 대신 비공개 iCal 주소, 반복 일정 인식), Phase 13(명령줄 Tab 자동완성 — 명령어/채널명, Calendar 패널 실데이터 연결 + 좁은 화면 패널 전환 버그 수정), Phase 14(WASM Component Model 기반 플러그인 런타임 — 샌드박스 생명주기, fuel/메모리 제한, `cargo-component`로 빌드한 실제 예제 플러그인 3종으로 검증), Phase 15(데몬 모드 & 로컬 CLI 소켓 IPC — `termws slack-send`/`set-presence`/`status`, v1.0.0의 마지막 기능 항목)까지 구현되어 있습니다 — 각 단계가 무엇을 다루고 왜 그렇게 했는지는 [`docs/07-implementation-log/`](docs/07-implementation-log/)의 `step2.md` ~ `step15.md`를 참고하세요.

v1.0.0 릴리스 스코프(`docs/01-product/product-requirements.md` §4) 중 기능 항목은 모두 구현되었습니다 — 원래 계획에는 실제 공개 릴리스 태그/발표만 남아 있었습니다.

**v1.0.0 스코프 밖에서 이후에 추가된 것들** (실사용 피드백 기반): Phase 16(플러그인 `get-member-presence` + 실제 capability 강제), Phase 17(로그 패널 — 이후 Phase 19/20에서 오버레이로 재설계), Phase 18(Pomodoro 타이머), Phase 19(UI 전반 재검토 — 색상 코딩, 패널 카운트 표시, 도움말 문서 보완), Phase 20(80칸 최소 터미널에서 헤더가 잘리던 실제 버그 수정), Phase 21(데스크톱 OS 알림), Phase 22(터미널 탭/창 제목에 안 읽음 개수 표시). 남은 건 여전히 AI Assistant 패널과 실제 공개 릴리스입니다 — 진행 상황은 [`docs/07-implementation-log/`](docs/07-implementation-log/)의 `step16.md` ~ `step22.md`를 참고하세요.

## 문서

전체 아키텍처, 설계 결정, 명세는 [`docs/`](docs/README.md)에 있습니다 — "어떻게 실행하는지"를 넘어서는 내용은 여기서부터 보시면 됩니다.

## 개발

- `cargo check --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo fmt --check` / `cargo test --workspace`
- 코드 스타일, 기능 변경 절차, 이 코드베이스가 따르는 Architecture Freeze v1 규칙은 [`docs/06-development/development.md`](docs/06-development/development.md)를 참고하세요.
