// Drives one conversation in the portal (app/ on $PORTAL) and saves screenshots.
import { chromium } from 'playwright';
const portal = process.env.PORTAL ?? 'http://localhost:5302';
const out = process.env.OUT ?? 'test/out';
const question = 'What happens when I go over the rate limit?';

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
await page.goto(portal);
await page.getByTestId('page').waitFor();
const box = page.locator('aside [contenteditable="true"]').first();
await box.waitFor({ timeout: 30000 });
await page.screenshot({ path: `${out}/1-portal.png` });
await box.click();
await page.keyboard.type(question);
await page.keyboard.press('Enter');
// The agent searches the docs and opens the page it used.
try {
  await page.locator('[data-testid="page"][data-slug="rate-limits"]').waitFor({ timeout: 120000 });
} catch (e) {
  await page.screenshot({ path: `${out}/failed.png` });
  console.error(await page.locator('aside').innerText());
  throw e;
}
await page.waitForTimeout(3000);
await page.screenshot({ path: `${out}/2-answered.png` });
const transcript = await page.locator('aside').innerText();
await browser.close();
console.log(transcript);
if (!/429|60 requests/i.test(transcript)) {
  console.error('the answer does not quote the rate-limits page');
  process.exit(1);
}
