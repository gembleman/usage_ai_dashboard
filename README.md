# 코딩 AI 사용량 대시보드

Codex, Claude Code, pi 및 OpenCode 로컬 세션 사용량을 계정 / 날짜 / 모델별로 집계합니다.
비슷한 게 이미 발에 차고 넘치지만, 그냥 만들었습니다.  

## 이용법
config.toml 만들기.  

```toml
#예시)
[server]
host = "127.0.0.1"
port = 4675
frontend_dir = "src/frontend" # config.toml 기준, 바이너리에 포함되지 않음

[dashboard]
page_size = 50
model_chart_max_items = 8
auto_refresh_seconds = 0 # 0이면 비활성화

[cache]
path = "cache.sqlite3" # 상대 경로는 config.toml 기준

[timeouts]
api_seconds = 30
refresh_seconds = 120
anthropic_seconds = 8

# 모델별 100만 토큰당 USD 가격
# GPT-6 Sol/Luna의 Standard 단기 컨텍스트 요금은 기본 제공되며, 아래 항목으로 덮어쓸 수 있습니다.
[model_pricing]
"claude-opus-5" = { input = 5.0, cached_input = 0.5, cache_creation_input = 6.25, output = 25.0 }
"claude-opus-4-8" = { input = 5.0, cached_input = 0.5, cache_creation_input = 6.25, output = 25.0 }
"claude-opus-4-7" = { input = 5.0, cached_input = 0.5, cache_creation_input = 6.25, output = 25.0 }
"claude-sonnet-5" = { input = 3.0, cached_input = 0.3, cache_creation_input = 3.75, output = 15.0 }
"claude-sonnet-4-6" = { input = 3.0, cached_input = 0.3, cache_creation_input = 3.75, output = 15.0 }
"claude-fable-5" = { input = 10.0, cached_input = 1.0, cache_creation_input = 12.5, output = 50.0 }
"claude-haiku-4-5" = { input = 1.0, cached_input = 0.1, cache_creation_input = 1.25, output = 5.0 }
"gpt-6-sol" = { input = 2.0, cached_input = 0.2, cache_creation_input = 2.5, output = 10.0 }
"gpt-6-luna" = { input = 0.1, cached_input = 0.01, cache_creation_input = 0.125, output = 0.5 }
"gpt-5.6-sol" = { input = 5.0, cached_input = 0.5, cache_creation_input = 6.25, output = 30.0 }
"gpt-5.6-terra" = { input = 2.5, cached_input = 0.25, cache_creation_input = 3.125, output = 15.0 }
"gpt-5.5" = { input = 5.0, cached_input = 0.5, cache_creation_input = 6.25, output = 30.0 }
"gpt-5.4" = { input = 2.5, cached_input = 0.25, cache_creation_input = 3.125, output = 15.0 }
"gpt-5.4-mini" = { input = 0.75, cached_input = 0.075, cache_creation_input = 0.9375, output = 4.5 }
"gpt-5.4-nano" = { input = 0.2, cached_input = 0.02, cache_creation_input = 0.25, output = 1.25 }
"gpt-4.1" = { input = 2.0, cached_input = 0.2, cache_creation_input = 2.5, output = 8.0 }

[[codex_accounts]]
name = "user1"
codex_home = "~/.codex"
# refresh = false # 이 계정은 수집하지 않고 캐시에 저장된 기존 데이터만 유지

[[claude_accounts]]
name = "user2"
config_dir = "~/.claude"
include_subagents = true

[[pi_accounts]]
name = "user3"
pi_home = "~/.pi"

[[opencode_accounts]]
name = "user4"
data_dir = "~/.local/share/opencode"

# 별도 token-usage-api 서버의 /records 데이터도 함께 수집
[[token_api_servers]]
name = "server-169" # 대시보드에 표시할 계정명
url = "http://192.168.1.169:8787"
# refresh = false
```

각 `codex_accounts`, `claude_accounts`, `pi_accounts`, `opencode_accounts`,
`token_api_servers` 항목에서 `refresh = false`로 지정하면 해당 계정은 시작 시
수집과 수동/자동 새로고침 대상에서 제외됩니다. SQLite 캐시에 이미 저장된
데이터는 삭제되지 않아 대시보드에 계속 표시됩니다. 이 항목을 생략하면
기본값은 `true`입니다.

### OpenCode 사용량에 대해

OpenCode의 현재 SQLite 저장소(`opencode*.db`)와 예전 JSON 저장소
(`storage/message/**/*.json`)를 모두 지원합니다. 업그레이드 뒤 두 형식에 같은
메시지가 남아 있어도 메시지 ID로 중복 제거하며, 세션 포크가 새 ID로 복사한
과거 호출도 원래 호출 시각과 사용량을 기준으로 한 번만 집계합니다.

OpenCode가 메시지마다 기록한 비용을 그대로 사용하므로 OpenCode 모델을
`model_pricing`에 추가할 필요가 없습니다. OpenCode도 로컬 로그에 요청 한도
정보를 남기지 않으므로 한도 패널에는 표시되지 않습니다.

### pi 사용량에 대해

pi는 서드파티 provider(opencode-go 등)를 거치며, 세션 로그에 실제 청구액을
직접 기록합니다. 따라서 pi 레코드의 비용은 `model_pricing` 표로 추정하지 않고
로그에 기록된 값을 그대로 사용합니다. pi가 쓰는 모델(deepseek, glm, kimi 등)을
`model_pricing`에 추가할 필요가 없습니다.

pi는 세션을 재개하면 이전 대화를 새 세션 파일로 복사하므로, 같은 메시지가
여러 파일에 중복 기록됩니다. 파서는 레코드 `id` 기준으로 계정 전체에 걸쳐
중복을 제거합니다(그러지 않으면 실제 로그 기준 약 48% 과다 집계됩니다).

pi는 로그에 요청 한도(rate limit) 정보를 남기지 않으므로 한도 패널에는
표시되지 않습니다.

```
./usage_ai_dashboard.exe serve
```

프론트엔드 파일은 `server.frontend_dir`에서 실행 중에 읽습니다. 따라서 HTML,
CSS, JavaScript를 수정할 때 Rust를 다시 컴파일할 필요가 없습니다. 배포 시에는
이 디렉터리도 실행 파일 및 `config.toml`과 함께 복사해야 합니다.

기간 필터에서 특정 월, 주 또는 날짜를 선택하면 해당 기간의 사용량을 모든
사용량 패널과 상세 내역에 적용합니다. 월과 주 목록은 기록이 있는 기간에서 만듭니다.
주간은 브라우저 현지 날짜를 기준으로 월요일부터 일요일까지입니다.
그래프 색상은 계정 또는 모델 이름을 기준으로 고정됩니다.

### 지원 브라우저

Chromium 153만 지원.

## 통계를 보고 얻은 통찰
6개월간 사용량이 대략 100억 토큰.  
2025년부터 써왔으니, 대략 200억 토큰 정도 쓰지 않았을까 싶습니다.  
웹으로 제공하는, claude와 chatgpt 토큰 수는 포함 안 했으니, 그것도 더하면 훨씬 많을 테죠.  
이 100억 토큰을 api 요금으로 계산해보니, 약 7000달러.  
6개월 간 구독을 통해 소비한 돈은 480달러. 효율은 16배?

chatGPT 모델과 비교했을 때, 확실히 claude 모델이 토큰을 5배 이상 많이 씁니다.  
코덱스를 쓰기 시작한지 1달 정도. 나름 괜찮아서, 적극적으로 쓰게 됐습니다.  
벌써 총 비용 중 9%는 gpt-5.5가 차지합니다.  
