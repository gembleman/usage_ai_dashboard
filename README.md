# 코딩 AI 사용량 대시보드

Codex 및 Claude Code 로컬 세션 사용량을 계정 / 날짜 / 모델별로 집계합니다.  
비슷한 게 이미 발에 차고 넘치지만, 그냥 만들었습니다.  

## 이용법
config.toml 만들기.  

```toml
#예시)
[server]
host = "127.0.0.1"
port = 4675

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
[model_pricing]
"claude-opus-4-8" = { input = 5.0, cached_input = 0.5, cache_creation_input = 6.25, output = 25.0 }
"claude-opus-4-7" = { input = 5.0, cached_input = 0.5, cache_creation_input = 6.25, output = 25.0 }
"claude-sonnet-5" = { input = 3.0, cached_input = 0.3, cache_creation_input = 3.75, output = 15.0 }
"claude-sonnet-4-6" = { input = 3.0, cached_input = 0.3, cache_creation_input = 3.75, output = 15.0 }
"claude-fable-5" = { input = 10.0, cached_input = 1.0, cache_creation_input = 12.5, output = 50.0 }
"claude-haiku-4-5" = { input = 1.0, cached_input = 0.1, cache_creation_input = 1.25, output = 5.0 }
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

[[claude_accounts]]
name = "user2"
config_dir = "~/.claude"
include_subagents = true
```

```
./usage_ai_dashboard.exe serve
```

### 지원 브라우저

Chrome, Edge, Firefox, Safari의 최신 안정 버전만 지원합니다. 대시보드는
`Temporal`, `Intl.DurationFormat`, `Map.groupBy`, `AbortSignal.timeout` 및
`AbortSignal.any`를 폴리필 없이 사용하므로 구형 브라우저는 지원하지 않습니다.

## 통계를 보고 얻은 통찰
6개월간 사용량이 대략 100억 토큰.  
2025년부터 써왔으니, 대략 200억 토큰 정도 쓰지 않았을까 싶습니다.  
웹으로 제공하는, claude와 chatgpt 토큰 수는 포함 안 했으니, 그것도 더하면 훨씬 많을 테죠.  
이 100억 토큰을 api 요금으로 계산해보니, 약 7000달러.  
6개월 간 구독을 통해 소비한 돈은 480달러. 효율은 16배?

chatGPT 모델과 비교했을 때, 확실히 claude 모델이 토큰을 5배 이상 많이 씁니다.  
코덱스를 쓰기 시작한지 1달 정도. 나름 괜찮아서, 적극적으로 쓰게 됐습니다.  
벌써 총 비용 중 9%는 gpt-5.5가 차지합니다.  
