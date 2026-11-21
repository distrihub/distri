# customer-service: the use case

1. `distri apply samples/customer-service --set business_name="Acme Coffee" --wait`: open the Telegram link it prints and press Start.
2. `distri apply samples/customer-service --set business_name="Acme Coffee" --test`: the test message (Sam, charged twice for order #1042) is triaged `escalate`; Telegram shows `🚨 Needs you: …` with the gist and the draft reply.
3. Run the workflow with a question it can answer (for example "Do you open on Sundays?"): Telegram shows `🧾 …` with the gist and the answer it sent.

`test/run.sh` runs 2 without Telegram: it checks the triage, the reply and the gist, and records whether the notify step was delivered. Step 1, and seeing the Telegram messages, need a person.
