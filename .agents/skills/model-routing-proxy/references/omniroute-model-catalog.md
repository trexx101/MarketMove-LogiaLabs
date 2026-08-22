# OmniRoute Auto-Routing Model Catalog

OmniRoute exposes these `auto/*` model IDs by default. Many have built-in routing that resolves to configured providers even without custom combos.

## Tier 1 — Free / Zero-Cost

| Model ID | Best For |
|----------|----------|
| `auto/best-free` | Best available free models across all categories |
| `auto/coding:free` | Free coding models (Qwen3 Coder free, DeepSeek V4 Flash free, etc.) |

## Tier 2 — Cheap (Everyday Work)

| Model ID | Best For |
|----------|----------|
| `auto/cheap` | Cheapest general-purpose model with good quality |
| `auto/coding:cheap` | Cheap-but-capable coding (day-to-day dev work) |
| `auto/coding:fast` | Low-latency coding (quick edits, interactive loops) |

## Tier 3 — Standard

| Model ID | Best For |
|----------|----------|
| `auto/coding` | Standard coding tasks |
| `auto/chat` | General conversation |
| `auto/fast` | Fast general-purpose Q&A |
| `auto/smart` | Balanced quality-and-speed |
| `auto/reasoning` | General reasoning tasks |

## Tier 4 — Pro / Premium

| Model ID | Best For |
|----------|----------|
| `auto/coding:pro` | Complex architecture, major refactors |
| `auto/reasoning:pro` | Deep analysis, system design |
| `auto/best-coding` | Best overall coding model available |
| `auto/best-reasoning` | Strongest reasoning model |
| `auto/pro-coding` | Pro-tier coding (may be more expensive) |
| `auto/pro-reasoning` | Pro-tier reasoning |
| `auto/pro-fast` | Fast pro-tier |
| `auto/pro-chat` | Pro-tier chat |
| `auto/pro-vision` | Pro-tier vision |

## Tier 5 — Vision / Multimodal

| Model ID | Best For |
|----------|----------|
| `auto/vision` | Image analysis, vision tasks |
| `auto/best-vision` | Best vision model available |
| `auto/multimodal` | Multimodal understanding |
| `auto/pro-vision` | Premium vision |

## Provider-Specific

| Model ID Format | Example | Source |
|-----------------|---------|--------|
| `openrouter/<model>` | `openrouter/qwen/qwen3-coder:free` | OpenRouter |
| `opencode-go/<model>` | `opencode-go/deepseek-v4-pro` | OpenCode Go |
| `oc/<model>` | `oc/big-pickle` | OpenCode |
| `aug/<model>` | `aug/claude-sonnet-4.6` | Auggie |
| `gemini/<model>` | `gemini/gemini-2.5-flash` | Google Gemini |
| `tllm/<model>` | `tllm/CLAUDE_4_6_SONNET` | The Old LLM |
| `ddgw/<model>` | `ddgw/gpt-4o-mini` | DuckDuckGo Web |
| `mcode/<model>` | `mcode/mimo-auto` | MimoCode |

## Fast Model Families

| Model ID | Best For |
|----------|----------|
| `auto/best-fast` | Best fast model overall |
| `auto/fast` | Fast general-purpose |
| `auto/pro-fast` | Fast pro-tier |
| `auto/coding:fast` | Fast coding |

## Claude-Specific

| Model ID | Description |
|----------|-------------|
| `auto/claude-opus` | Routes to Claude Opus tier |
| `auto/claude-sonnet` | Routes to Claude Sonnet tier |

## Usage from Hermes

Set any of these as the model in Hermes config, or switch mid-session:

```
/model custom:omniroute:auto/coding:cheap
/model custom:omniroute:auto/best-coding
/model custom:omniroute:auto/fast
```

The `custom:omniroute` part assumes you have a custom provider named `omniroute` pointing at `http://localhost:20128/v1`.

## Provider Backend Details

These are the main backend provider groups OmniRoute can route to:

| Provider Prefix | Source | Typical Models |
|----------------|--------|---------------|
| `openrouter/` | OpenRouter API | Qwen, Claude, GPT, Gemini, Llama via OpenRouter |
| `opencode-go/` | OpenCode Go | DeepSeek V4, Qwen 3.5/3.6/3.7, MiMo V2, Kimi K2.5/2.6, GLM-5, Hunyuan |
| `aug/` | Auggie | Claude Sonnet 4.6, Opus 4.6, Haiku 4.5, Gemini 3.1 Pro, GPT-5.5 |
| `oc/` | OpenCode | Big Pickle, DeepSeek V4 Flash Free, MiniMax M3 Free |
| `gemini/` | Google Gemini | Gemini 2.5 Flash, 3 Flash Preview, 3.1 Flash Lite |
| `tllm/` | The Old LLM (free) | Claude 4.6 Opus/Sonnet, DeepSeek V4, Gemini 3 Flash |
| `ddgw/` | DuckDuckGo Web | GPT-4o Mini, GPT-5 Mini, Claude 3.5 Haiku, Llama 4 Scout |
| `no-think/` | Auggie (no thinking) | Claude Opus/Sonnet/Haiku without extended thinking |
