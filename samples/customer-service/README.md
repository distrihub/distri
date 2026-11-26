# Customer service desk with Telegram escalation

> **"Create a customer service agent for my shop that answers customer messages, sends me a short gist of each one on Telegram, and escalates refunds and angry customers to me there."**

## What it sets up

| What | Name | Notes |
|---|---|---|
| Agent | `support_desk` | Server agent. For each message it returns JSON: `decision` (`escalate` or `answer`), the `reply` to send, and a one-line `gist`. |
| Workflow | `handle_message` | Runs the agent on `{from, message}`, then sends you `🚨 Needs you: …` with the draft reply, or `🧾 …` with the answer it gave. |
| Chat | Telegram | Connect with one link. The gists and escalations arrive there, and you can ask `support_desk` questions in the same chat. |

```
{from, message} ─► handle (support_desk → {decision, reply, gist}) ─┬─ escalate ─► 🚨 Telegram: gist + draft reply
                                                                    └─ answer ───► 🧾 Telegram: gist + answer
```

What counts as an escalation is the `escalate_when` param (default: refunds, double charges, complaints, legal questions, and anything the agent cannot answer).

## Run it

```bash
distri login
distri apply samples/customer-service --set business_name="Acme Coffee" --wait --test
```

`--wait` waits while you open the Telegram link it prints. `--test` then sends the test message ("I was charged twice for order #1042. I want a refund today.") through the workflow, and an `🚨 Needs you` message arrives on Telegram.

Without Telegram the run still finishes. The notify step records `notify: delivered nowhere: no chat connected for the owner`, and `test/out/` holds the decision, the reply and the gist.

## Test

`DISTRI_URL=… DISTRI_API_KEY=… test/run.sh` applies the folder, runs the test message, checks that it was escalated with a reply and a gist, and records whether Telegram got it. The steps that need a person (connecting Telegram, reading the message) are in [`test/case.md`](test/case.md).
