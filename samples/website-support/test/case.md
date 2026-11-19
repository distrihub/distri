# Use case: website support agent from one sentence

**As:** a fresh sign-up (`fresh` account) with a Linear test workspace.

1. In web chat, type: *Create an onboarding agent for my product on `<demo-site>` that answers questions from our docs and opens a support ticket when it can't help.*
2. Confirm the card, then connect Linear and Telegram from the links.
3. Paste the snippet into the demo page and open it.
4. Ask a question the docs answer, then one they don't. Give an email when asked.

**Expect:**
- the docs question gets an answer with a link to the docs page;
- the other question gets "I've passed this to the team", a Linear issue with the visitor's email exists, and a 🎫 message arrives on Telegram;
- the browser never receives a Linear token (check the network tab).
