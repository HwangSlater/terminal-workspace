# Terminal Workspace

터미널을 벗어나지 않고 Slack, GitHub, Google Calendar를 한곳에서 확인하고 다루는 터미널 우선 개발자 워크스페이스입니다. (Gmail, Jira, CI/CD 연동은 로드맵에 있지만 아직 준비되지 않았습니다.)

Local First. Zero Configuration. Windows·macOS·Linux 어느 OS도 2등 시민 취급하지 않는 크로스 플랫폼 — 자세한 내용은 [`docs/06-development/platform-support.md`](docs/06-development/platform-support.md) 참고.

---

## 시작하기

**1. Rust 설치** (아직 없으시다면): <https://rustup.rs> — 이것만 있으면 됩니다. C 컴파일러도, 별도 데이터베이스 서버도, 추가 툴체인도 필요 없습니다.

**2. 실행:**

```sh
cargo run -p app
```

설정 파일을 손으로 작성할 필요가 없습니다 — 처음 실행하면 `config.toml`과 로컬 데이터베이스가 자동으로 만들어집니다.

선택: `scripts/setup.ps1`(Windows) / `scripts/setup.sh`(Linux/macOS)로 환경을 한 번에 점검할 수 있습니다 — `rustup` 존재 여부 확인, 빠진 게 있으면 정확한 설치 명령어 안내, `cargo check --workspace`까지 한 번에 해줍니다.

### Windows

대부분의 경우 위 두 단계로 끝입니다. 만약 아래 오류를 만나면:

```
error: error calling dlltool 'dlltool.exe': program not found
```

GCC 기반 MinGW-w64를 설치하세요: `winget install BrechtSanders.WinLibs.POSIX.UCRT` (설치 프로그램 없이 압축 해제만 하는 방식이라 가볍습니다). **LLVM 기반 MinGW은 피하세요** — Rust가 기대하는 `libgcc`/`libgcc_eh`가 없어 링크가 실패합니다. 설치 후 `mingw64\bin` 폴더를 PATH에 추가하고 새 터미널에서 다시 시도하세요.

### macOS

Xcode Command Line Tools가 필요합니다 — 이미 있는지 `xcode-select -p`로 확인, 없으면 `xcode-select --install`. Slack 토큰은 macOS 키체인(Keychain Services)에 자동으로 저장됩니다.

### Linux

HTTP 통신에 시스템 OpenSSL을 씁니다 (보통 데스크톱 배포판엔 이미 있습니다):

- Debian/Ubuntu: `sudo apt install build-essential libssl-dev pkg-config`
- Fedora/RHEL: `sudo dnf install gcc openssl-devel pkgconf-pkg-config`
- Arch: `sudo pacman -S base-devel openssl pkgconf`

Slack 토큰은 데스크톱 환경(GNOME/KDE 등)이면 자동으로 키체인(gnome-keyring/kwallet)에 저장되고, 헤드리스/서버 환경이면 로컬 암호화 파일로 자동 대체 저장됩니다 — 별도 조치가 필요 없습니다.

---

## 주요 기능

앱을 실행하면(`cargo run -p app`) 터미널 전체 화면을 쓰는 대화형 UI가 뜹니다. 키보드로만 조작합니다 (Vim에서 영감을 받은 모달 입력 방식):

| 키 | 동작 |
| :--- | :--- |
| `Tab` / `Shift+Tab` | 패널 포커스를 순서대로 이동 (팀 → 알림 → 캘린더) |
| `Ctrl+1` ~ `Ctrl+3` | 팀 / 알림 / 캘린더 패널로 바로 이동 |
| `Ctrl+4` | 로그 보기 오버레이 열기 |
| `j`/`k`, `↑`/`↓` | 포커스된 패널 안에서 위아래로 선택 이동 |
| `:` | 명령줄 입력 모드로 전환 |
| `?` | 도움말 팝업 열기 |
| `Esc` | 명령줄/도움말/오버레이 닫고 Normal 모드로 복귀 |
| `Ctrl+S` / `Ctrl+P` | Slack 연결 설정 / 채널·사용자 선택 |
| `Ctrl+G` / `Ctrl+R` | GitHub 연결 설정 / 저장소 선택 |
| `Ctrl+L` / `Ctrl+K` | Calendar 추가 / 연결된 캘린더 관리·제거 |
| `Ctrl+Q` | 종료 |

### Slack

1. Slack 워크스페이스에 App을 하나 만들고, Bot Token 스코프로 `channels:history`, `channels:read`, `users:read`, `chat:write`를 추가한 뒤 설치해 Bot Token(`xoxb-...`)을 발급받으세요.
2. `Ctrl+S`로 토큰을 입력하면 바로 연결됩니다 — 토큰은 OS 키체인(또는 암호화 파일)에 저장되고 설정 파일에는 절대 들어가지 않습니다.
3. 메시지를 받을 채널에 봇을 초대하세요 (`/invite @봇이름`).
4. `Ctrl+P`로 채널/팀원 목록을 불러와 선택하세요.
5. 명령줄(`:`)에서 바로 사용할 수 있습니다: `/send #채널이름 메시지`, `/away`·`/active`·`/offline`·`/meeting`·`/lunch [메시지]`로 상태 변경. 명령어/채널명 입력 중 `Tab`으로 자동완성됩니다.

자세한 내용은 [`docs/04-extensions/integrations/slack.md`](docs/04-extensions/integrations/slack.md) 참고.

### GitHub

1. GitHub → Settings → Developer settings → Personal access tokens에서 `repo` 스코프로 Classic PAT(`ghp_...`)를 발급받으세요.
2. `Ctrl+G`로 토큰을 입력하면 바로 연결됩니다.
3. `Ctrl+R`로 접근 가능한 저장소를 선택하면, 열린 PR이 알림 패널에 표시됩니다.

자세한 내용은 [`docs/04-extensions/integrations/github.md`](docs/04-extensions/integrations/github.md) 참고.

### Calendar

1. Google Calendar → 설정 → 캘린더 통합 → **비공개 iCal 형식 주소**를 복사하세요. OAuth나 로그인 절차가 필요 없습니다.
2. `Ctrl+L`로 이 캘린더를 부를 이름(예: "회사")을 입력하고 Enter, 이어서 주소를 입력하면 바로 연결됩니다. 앞으로 24시간(설정으로 조절 가능) 이내 일정이 알림 패널에 `[회사] 회의 이름`처럼 이름과 함께 표시되고, 반복 일정도 인식합니다.
3. 여러 캘린더(예: 회사 + 개인)를 동시에 연결할 수 있습니다 — `Ctrl+L`을 다시 눌러 하나씩 추가하세요.
4. `Ctrl+K`로 연결된 캘린더 목록을 보고, 제거하고 싶은 걸 체크 해제한 뒤 저장하면 됩니다.

자세한 내용은 [`docs/04-extensions/integrations/calendar.md`](docs/04-extensions/integrations/calendar.md) 참고.

### Pomodoro 타이머

명령줄에서 `/pomodoro start [작업분] [휴식분]`(기본 25/5분), `/pomodoro pause`, `/pomodoro reset`으로 조작합니다. 헤더에 남은 시간이 실시간으로 표시되고(`🍅 24:35 (Work)`), 세션이 끝나면 터미널 벨이 울리고 자동으로 다음 모드로 전환됩니다.

### 로그 보기

`Ctrl+4`로 앱의 로그 기록(최근 200줄, 비밀값 자동 마스킹)을 오버레이로 확인할 수 있습니다. `ERROR`/`WARN` 줄은 색으로 구분됩니다.

### 알림을 놓치지 않으려면

Slack DM, GitHub PR 리뷰 요청, Calendar 리마인드, Pomodoro 세션 종료가 오면 다른 터미널이나 다른 앱을 보고 있어도 알아챌 수 있도록 두 가지로 알려줍니다:

- **데스크톱(OS) 토스트 알림** — Windows/macOS/Linux 어디서든 화면에 시스템 팝업이 뜹니다.
- **터미널 탭/창 제목** — 안 읽은 알림이 있으면 탭 제목이 `Terminal Workspace (3)`처럼 바뀝니다. 다른 탭에서 작업 중이어도 탭 바에서 바로 보입니다.

둘 다 별도 설정 없이 항상 켜져 있고, 실패해도(예: 알림 데몬이 없는 헤드리스 Linux) 앱 자체는 영향받지 않습니다.

### 플러그인 (실험적)

WebAssembly 기반 샌드박스 플러그인 런타임입니다. 기본적으로 꺼져 있고, `config.toml`의 `[plugins]`에서 `enabled = true`로 켜고 `directory`/`allowed_list`를 채워야 로드됩니다. 매 이벤트마다 CPU/메모리 제한이 걸려 있어, 문제가 있는 플러그인이 있어도 워크스페이스 전체는 영향받지 않습니다. 플러그인 작성은 [`docs/04-extensions/plugin-lifecycle.md`](docs/04-extensions/plugin-lifecycle.md), `examples/plugins/hello` 참고.

### 데몬 모드 & CLI

앱을 실행하면 그 인스턴스 자체가 데몬입니다 — 별도 프로세스 설치 없이 다른 터미널에서 짧은 명령으로 접근할 수 있습니다:

```sh
cargo run -p app -- slack-send '#general' 안녕하세요
cargo run -p app -- set-presence away 자리 비움
cargo run -p app -- status
```

자세한 내용은 [`docs/01-product/user-flows.md`](docs/01-product/user-flows.md) §3 참고.

---

## 문서

전체 아키텍처, 설계 결정, 명세는 [`docs/`](docs/README.md)에 있습니다. 각 기능이 언제·왜·어떻게 만들어졌는지의 개발 이력은 [`docs/07-implementation-log/`](docs/07-implementation-log/)에서 확인할 수 있습니다.

## 개발

- `cargo check --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo fmt --check` / `cargo test --workspace`
- 코드 스타일, 기능 변경 절차, 이 코드베이스가 따르는 Architecture Freeze v1 규칙은 [`docs/06-development/development.md`](docs/06-development/development.md)를 참고하세요.
