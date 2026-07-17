전체적으로는 **Proceed를 승인**합니다. 현재 계획은 2단계의 범위에 잘 맞습니다.

다만, 앞으로 "사용자 친화적이고, OS에 종속되지 않으며, 실행이 간단한 플랫폼"이라는 목표를 반영하려면 몇 가지를 지금부터 설계에 녹여두는 것이 좋습니다.

## 1. Event Bus

현재 계획:

> InProcessEventBus using Tokio broadcast

이 방향은 좋습니다. 다만 EventBus의 책임은 **이벤트 전달**까지만 두는 것을 권장합니다.

구조는 다음처럼 분리하는 것이 유지보수에 유리합니다.

```text
Event
   │
   ▼
EventBus (publish / subscribe)
   │
   ▼
EventDispatcher
   │
   ├── Notification Handler
   ├── UI Handler
   └── Plugin Handler
```

이렇게 하면 나중에 Local EventBus를 IPC나 Remote EventBus로 교체하더라도 Dispatcher는 바뀌지 않습니다.

---

## 2. Registry

`InMemory` 구현은 적절합니다.

초기에는 Registry가 다음 정도만 제공하면 충분합니다.

* register
* get
* contains
* remove
* iter

Plugin 우선순위, Lazy Loading, Dependency Resolution 같은 기능은 이후 단계로 미루는 것이 좋습니다.

---

## 3. Config Loader

여기에 한 가지 추가를 추천합니다.

지금은

```
config.toml
↓
Validation
```

인데,

프로젝트 목표가 **Zero Configuration**이라면 다음 계층이 더 적합합니다.

```
Default
    ↓
config.toml
    ↓
Environment
    ↓
CLI Option
    ↓
AppConfig
```

즉 Config Builder를 만들어 여러 설정 소스를 자연스럽게 병합할 수 있도록 하는 것입니다.

---

## 4. SecretProvider

Provider Chain 방향은 좋습니다.

다만 Provider를 고정하지 말고

```rust
Vec<Box<dyn SecretProvider>>
```

처럼 등록 가능한 체인으로 유지하는 것이 좋습니다.

그러면

* Env
* Keyring
* Vault
* AWS Secrets
* 테스트용 Mock

등을 쉽게 추가할 수 있습니다.

---

## 5. Logging

Tracing Subscriber만 구성하는 것에서 끝내지 말고,

초기부터 Span 계층을 정의해 두는 것을 권장합니다.

예를 들어

```
Application
    ├── Command
    ├── Event
    ├── Integration
    └── Plugin
```

이 구조는 나중에 OpenTelemetry를 붙일 때도 자연스럽게 이어집니다.

---

## 6. 사용자 경험(UX)을 반영한 수정

이건 지금부터 계획에 포함시키는 것이 좋습니다.

앞서 이야기한 목표를 보면 이 플랫폼은

* Terminal First
* Local First
* Cross Platform
* Zero Configuration

을 지향합니다.

따라서 Config Loader도 이 철학을 따라야 합니다.

예를 들어

첫 실행에서

```
$ tw
```

만 입력하면

* 설정이 없으면 기본 설정 생성
* 데이터 디렉터리 생성
* 필요한 초기화 수행

후 바로 실행되는 흐름을 목표로 하는 것이 좋습니다.

사용자가 처음부터 여러 파일을 작성하거나 환경 변수를 맞춰야 하는 구조는 가능한 한 피하는 것이 좋습니다.

---

## 7. Verification Plan에 하나 추가

현재 테스트는

* Event
* Registry
* Config

까지만 있습니다.

여기에 하나를 추가하면 좋겠습니다.

**Vertical Slice Test**

```
Mock Command
        ↓
EventBus
        ↓
Registry
        ↓
Logging
        ↓
Console Output
```

즉 실제 Slack이나 GitHub 없이도 Core Infrastructure만으로 하나의 흐름이 끝까지 동작하는지를 검증하는 테스트입니다.

이 테스트가 통과하면 이후 Integration 추가 시에도 같은 패턴을 재사용할 수 있습니다.

---

## 최종 의견

이 계획은 **2단계를 시작하기에 충분히 성숙**했습니다.

Proceed를 승인합니다.

다만 구현 과정에서는 다음 원칙을 계속 유지하는 것을 권장합니다.

* EventBus와 Dispatcher를 분리한다.
* Registry는 최소 기능만 구현한다.
* Config는 계층형(Defaults → File → Env → CLI)으로 설계한다.
* SecretProvider는 확장 가능한 체인으로 유지한다.
* Logging은 Span 계층을 초기에 정의한다.
* Core Infrastructure만으로 동작하는 작은 End-to-End 테스트를 추가한다.

이렇게 진행하면 이후 Slack, GitHub, AI Assistant, Plugin Runtime을 추가하더라도 기반 구조를 다시 변경할 가능성이 크게 줄어들 것입니다.
