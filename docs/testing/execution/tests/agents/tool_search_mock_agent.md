---
name = "tool_search_mock_agent"
version = "1.0.0"
description = "Stress-tests the deferred-loading + tool_search path. The agent has 25+ inline mock dynamic tools spanning weather/stock/translate/db/email/etc. — well past the deferred-loading threshold — plus `final` and `tool_search`. The user task names a domain (e.g. 'weather in Tokyo'); the agent must use tool_search to discover the right tool, call it, and return its response. Each mock tool's description, parameter schema, and canned response are defined inline below."
append_default_instructions = false
max_iterations = 8
tool_format = "provider"
tool_delivery_mode = "tool_search"

[tools]
builtin = ["final", "tool_search"]

[[tools.dynamic]]
name = "weather_lookup"
type = "mock"
[tools.dynamic.config]
description = "Get the current weather for a city. Returns temperature in Celsius, conditions, humidity, and wind."
parameters = { type = "object", properties = { city = { type = "string" }, country = { type = "string" } }, required = ["city"] }
response = { city = "Tokyo", temperature_c = 14.2, conditions = "partly cloudy", humidity_pct = 62, wind_kph = 11 }

[[tools.dynamic]]
name = "stock_quote"
type = "mock"
[tools.dynamic.config]
description = "Look up the latest price for a stock ticker. Returns price, day-change, and volume."
parameters = { type = "object", properties = { ticker = { type = "string" } }, required = ["ticker"] }
response = { ticker = "AAPL", price_usd = 232.18, change_pct = -0.41, volume = 47193204 }

[[tools.dynamic]]
name = "currency_convert"
type = "mock"
[tools.dynamic.config]
description = "Convert an amount between two currencies using current FX rates."
parameters = { type = "object", properties = { amount = { type = "number" }, from = { type = "string" }, to = { type = "string" } }, required = ["amount", "from", "to"] }
response = { converted = 87.34, rate = 0.8734 }

[[tools.dynamic]]
name = "translate_text"
type = "mock"
[tools.dynamic.config]
description = "Translate text from one language to another."
parameters = { type = "object", properties = { text = { type = "string" }, source_lang = { type = "string" }, target_lang = { type = "string" } }, required = ["text", "target_lang"] }
response = { translation = "こんにちは世界" }

[[tools.dynamic]]
name = "geocode_address"
type = "mock"
[tools.dynamic.config]
description = "Convert a postal address to latitude/longitude coordinates."
parameters = { type = "object", properties = { address = { type = "string" } }, required = ["address"] }
response = { lat = 35.6762, lon = 139.6503, accuracy = "rooftop" }

[[tools.dynamic]]
name = "reverse_geocode"
type = "mock"
[tools.dynamic.config]
description = "Convert latitude/longitude into a human-readable address."
parameters = { type = "object", properties = { lat = { type = "number" }, lon = { type = "number" } }, required = ["lat", "lon"] }
response = { address = "1-1 Chiyoda, Tokyo 100-0001, Japan" }

[[tools.dynamic]]
name = "send_sms"
type = "mock"
[tools.dynamic.config]
description = "Send an SMS message to a phone number."
parameters = { type = "object", properties = { to = { type = "string" }, body = { type = "string" } }, required = ["to", "body"] }
response = { message_id = "sms_01HZX9", status = "queued" }

[[tools.dynamic]]
name = "send_email"
type = "mock"
[tools.dynamic.config]
description = "Send a transactional email."
parameters = { type = "object", properties = { to = { type = "string" }, subject = { type = "string" }, body = { type = "string" } }, required = ["to", "subject", "body"] }
response = { message_id = "eml_01HZX9", status = "sent" }

[[tools.dynamic]]
name = "calendar_create_event"
type = "mock"
[tools.dynamic.config]
description = "Create a calendar event."
parameters = { type = "object", properties = { title = { type = "string" }, start = { type = "string" }, end = { type = "string" }, attendees = { type = "array", items = { type = "string" } } }, required = ["title", "start", "end"] }
response = { event_id = "evt_42", status = "confirmed" }

[[tools.dynamic]]
name = "calendar_list_events"
type = "mock"
[tools.dynamic.config]
description = "List upcoming calendar events for a user within a time window."
parameters = { type = "object", properties = { user = { type = "string" }, from = { type = "string" }, to = { type = "string" } } }
response = { events = [{ id = "evt_42", title = "Standup", start = "2026-05-11T09:00:00Z" }] }

[[tools.dynamic]]
name = "db_query"
type = "mock"
[tools.dynamic.config]
description = "Run a read-only SQL query against the analytics warehouse."
parameters = { type = "object", properties = { sql = { type = "string" }, max_rows = { type = "integer" } }, required = ["sql"] }
response = { rows = [{ col = "value" }], row_count = 1 }

[[tools.dynamic]]
name = "knowledge_search"
type = "mock"
[tools.dynamic.config]
description = "Search the internal knowledge base for documents matching a query."
parameters = { type = "object", properties = { query = { type = "string" }, top_k = { type = "integer" } }, required = ["query"] }
response = { hits = [{ doc_id = "kb_018", title = "Onboarding", score = 0.91 }] }

[[tools.dynamic]]
name = "ticket_create"
type = "mock"
[tools.dynamic.config]
description = "Open a support ticket on behalf of the user."
parameters = { type = "object", properties = { subject = { type = "string" }, body = { type = "string" }, priority = { type = "string" } }, required = ["subject", "body"] }
response = { ticket_id = "T-1042", status = "open" }

[[tools.dynamic]]
name = "ticket_close"
type = "mock"
[tools.dynamic.config]
description = "Close an existing support ticket with a resolution note."
parameters = { type = "object", properties = { ticket_id = { type = "string" }, resolution = { type = "string" } }, required = ["ticket_id"] }
response = { ticket_id = "T-1042", status = "closed" }

[[tools.dynamic]]
name = "user_lookup"
type = "mock"
[tools.dynamic.config]
description = "Find a user by email or username and return their profile."
parameters = { type = "object", properties = { email = { type = "string" }, username = { type = "string" } } }
response = { id = "u_42", name = "Ada Lovelace", email = "ada@example.com" }

[[tools.dynamic]]
name = "feature_flag_eval"
type = "mock"
[tools.dynamic.config]
description = "Evaluate a feature flag for a given user/context."
parameters = { type = "object", properties = { flag = { type = "string" }, user_id = { type = "string" } }, required = ["flag"] }
response = { flag = "new_dashboard", value = true, variant = "treatment" }

[[tools.dynamic]]
name = "image_resize"
type = "mock"
[tools.dynamic.config]
description = "Resize an image to a target width/height and return the new URL."
parameters = { type = "object", properties = { url = { type = "string" }, width = { type = "integer" }, height = { type = "integer" } }, required = ["url"] }
response = { url = "https://cdn.example.com/resized/abc.png", bytes = 84211 }

[[tools.dynamic]]
name = "pdf_extract"
type = "mock"
[tools.dynamic.config]
description = "Extract plain text from a PDF document URL."
parameters = { type = "object", properties = { url = { type = "string" }, max_pages = { type = "integer" } }, required = ["url"] }
response = { text = "...extracted text...", pages = 12 }

[[tools.dynamic]]
name = "youtube_search"
type = "mock"
[tools.dynamic.config]
description = "Search YouTube for videos matching a query."
parameters = { type = "object", properties = { query = { type = "string" }, max_results = { type = "integer" } }, required = ["query"] }
response = { videos = [{ id = "dQw4w9", title = "Top result" }] }

[[tools.dynamic]]
name = "youtube_transcript"
type = "mock"
[tools.dynamic.config]
description = "Fetch the transcript for a YouTube video."
parameters = { type = "object", properties = { video_id = { type = "string" }, lang = { type = "string" } }, required = ["video_id"] }
response = { transcript = "...transcript text...", duration_s = 213 }

[[tools.dynamic]]
name = "github_create_issue"
type = "mock"
[tools.dynamic.config]
description = "Create a GitHub issue on a repository."
parameters = { type = "object", properties = { owner = { type = "string" }, repo = { type = "string" }, title = { type = "string" }, body = { type = "string" } }, required = ["owner", "repo", "title"] }
response = { number = 1337, url = "https://github.com/o/r/issues/1337" }

[[tools.dynamic]]
name = "github_search_code"
type = "mock"
[tools.dynamic.config]
description = "Search code across GitHub repositories."
parameters = { type = "object", properties = { q = { type = "string" }, language = { type = "string" } }, required = ["q"] }
response = { items = [{ path = "src/main.rs", url = "https://github.com/...", score = 0.7 }] }

[[tools.dynamic]]
name = "slack_post_message"
type = "mock"
[tools.dynamic.config]
description = "Post a message to a Slack channel."
parameters = { type = "object", properties = { channel = { type = "string" }, text = { type = "string" } }, required = ["channel", "text"] }
response = { ts = "1731512345.001100", ok = true }

[[tools.dynamic]]
name = "stripe_charge"
type = "mock"
[tools.dynamic.config]
description = "Create a Stripe payment charge."
parameters = { type = "object", properties = { amount_cents = { type = "integer" }, currency = { type = "string" }, customer = { type = "string" } }, required = ["amount_cents", "currency", "customer"] }
response = { id = "ch_3PvFv", status = "succeeded" }

[[tools.dynamic]]
name = "s3_put_object"
type = "mock"
[tools.dynamic.config]
description = "Upload bytes to an S3 bucket key."
parameters = { type = "object", properties = { bucket = { type = "string" }, key = { type = "string" }, body_b64 = { type = "string" } }, required = ["bucket", "key", "body_b64"] }
response = { etag = "\"d41d8cd98f00b204e9800998ecf8427e\"" }

[[tools.dynamic]]
name = "dns_lookup"
type = "mock"
[tools.dynamic.config]
description = "Resolve a hostname's DNS records (A/AAAA/CNAME/MX/TXT)."
parameters = { type = "object", properties = { hostname = { type = "string" }, record_type = { type = "string" } }, required = ["hostname"] }
response = { records = [{ type = "A", value = "93.184.216.34" }] }

[[tools.dynamic]]
name = "url_unshorten"
type = "mock"
[tools.dynamic.config]
description = "Resolve a shortened URL to its final destination."
parameters = { type = "object", properties = { url = { type = "string" } }, required = ["url"] }
response = { final_url = "https://example.com/very/long/path" }

[[tools.dynamic]]
name = "wiki_summary"
type = "mock"
[tools.dynamic.config]
description = "Get a 1-paragraph summary for a Wikipedia article."
parameters = { type = "object", properties = { title = { type = "string" }, lang = { type = "string" } }, required = ["title"] }
response = { title = "Tokyo", summary = "Tokyo is the capital of Japan..." }

[[tools.dynamic]]
name = "math_solve"
type = "mock"
[tools.dynamic.config]
description = "Solve a math expression or equation symbolically."
parameters = { type = "object", properties = { expression = { type = "string" } }, required = ["expression"] }
response = { result = "x = 7" }

[[tools.dynamic]]
name = "uuid_generate"
type = "mock"
[tools.dynamic.config]
description = "Generate a v4 UUID."
parameters = { type = "object", properties = {} }
response = { uuid = "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d" }
---

# Tool-search stress test

You have 30+ tools available, but only `final` and `tool_search` are loaded in your prompt. Every other tool is **deferred** — you only see its name + description, not its parameter schema. Your job is to handle the user's request by discovering and calling the right tool.

## Procedure

1. Read the user's request. Identify what kind of action it asks for (weather lookup, stock price, translation, DB query, sending email, etc.).

2. Call `tool_search` to find candidate tools:
   - **Keyword search:** `tool_search({query: "<keywords from the request>"})` returns matching tools (deferred ones come back without a schema).
   - **Schema lookup:** when you've decided which tool to use, call `tool_search({names: ["<tool_name>"]})` to fetch its full parameter schema. After this call the tool is fully loaded for the next turn.

3. Call the discovered tool with the parameters required by its schema. The tool returns a canned but realistic response.

4. Pass the tool's response (or a short summary of it) to `final({result: <summary>})`.

## Hard rules

- ONE keyword `tool_search` to scout, ONE name-based `tool_search` to load the schema, ONE call to the actual tool, ONE `final`. Four steps total.
- Don't try to invoke a tool whose schema you haven't loaded — the deferred form has no parameters in your context, so you'll send empty arguments.
- Don't reach for `final` with a hallucinated answer. If `tool_search` returns 0 hits, search again with different keywords; only fall back to `final({result: "no matching tool"})` after two failed searches.
