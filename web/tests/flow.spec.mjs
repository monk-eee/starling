import { expect, test } from "@playwright/test";
import { spawn } from "node:child_process";
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = fileURLToPath(new URL("../../", import.meta.url));
const binary = resolve(root, process.env.CARGO_TARGET_DIR ?? "target", "debug",
  process.platform === "win32" ? "starling.exe" : "starling");
if (process.env.STARLING_BROWSER_CHANNEL) test.use({ channel: process.env.STARLING_BROWSER_CHANNEL });

function launch(args, prefix, children) {
  const child = spawn(binary, args, { cwd: root, stdio: ["ignore", "pipe", "pipe"] });
  children.push(child);
  return new Promise((resolve, reject) => {
    let stdout = "", stderr = "";
    child.stdout.on("data", chunk => {
      stdout += chunk;
      const line = stdout.split(/\r?\n/).slice(0, -1).find(s => s.startsWith(prefix));
      if (line) resolve(line.slice(prefix.length));
    });
    child.stderr.on("data", chunk => { stderr += chunk; });
    child.once("error", reject);
    child.once("exit", code => reject(new Error(`CLI exited ${code}: ${stderr}`)));
  });
}

async function stopChild(child) {
  if (child.exitCode !== null || child.signalCode !== null || !child.pid) return;
  await new Promise(resolve => { child.once("exit", resolve); child.kill(); });
}

test("start, receive live metrics, reconnect, stop and reopen the same report offline", async ({ page, context, request }, info) => {
  const children = [];
  const errors = [];
  page.on("pageerror", e => errors.push(e.message));
  const frames = [];
  page.on("websocket", ws => {
    if (ws.url().endsWith("/runs/local/live")) ws.on("framereceived", ({ payload }) => frames.push(JSON.parse(String(payload))));
  });
  try {
    const target = await launch(["fixture", "--listen", "127.0.0.1:0"], "Fixture: ", children);
    const config = info.outputPath("local.toml");
    const report = info.outputPath("report");
    await mkdir(dirname(config), { recursive: true });
    await writeFile(config, (await readFile(join(root, "examples", "local.toml"), "utf8"))
      .replace("http://127.0.0.1:8081/mcp", target).replace("hello", "PRIVATE_BROWSER_PAYLOAD"));
    const url = await launch(["run", "--web", "--listen", "127.0.0.1:0", "--config", config, "--report", report],
      "Dashboard: ", children);
    await page.goto(url);
    const status = page.getByRole("status", { name: "Run status" });
    await expect(status).toHaveText("ready");
    const bird = page.getByRole("navigation").locator("img");
    await expect(bird).toBeVisible();
    const start = `${url}/api/v1/runs/local/start`;
    expect((await request.post(start)).status()).toBe(403);
    expect((await request.post(start, { headers: { Origin: "https://other.invalid", "X-Starling-Control": "1" } })).status()).toBe(403);
    const headers = { "X-Starling-Control": "1" };
    for (const data of [{ users: 9 }, { threads: 0 }, { duration_ms: 999 }, { duration_ms: 604800001 }, { wait_ms: 60001 }, { timeout_ms: 15001 }]) {
      expect((await request.post(start, { headers, data })).status()).toBe(422);
    }
    const users = page.getByRole("slider", { name: "Concurrent users" });
    const threads = page.getByRole("slider", { name: "Worker threads" });
    await expect(users).toHaveValue("2");
    await users.press("ArrowRight");
    await threads.press("Home");
    const duration = page.getByRole("spinbutton", { name: "Duration", exact: true });
    const unit = page.getByRole("combobox", { name: "Duration unit" });
    await duration.fill("6");
    await unit.selectOption({ label: "hours" });
    const wait = page.getByLabel("Wait between calls (ms)");
    await expect(wait).toHaveValue("0");
    await wait.fill("400");
    await page.getByLabel("Operation timeout (ms)").fill("2500");
    await page.getByRole("button", { name: "Start run" }).click();
    await expect(status).toHaveText("running");
    expect((await request.post(start, { headers, data: { users: 8 } })).status()).toBe(409);
    await expect(threads).toHaveCount(0);
    await expect.poll(() => frames.some(f => f.type === "tick" && f.open_interval?.calls > 0)).toBe(true);
    await expect(page.getByRole("rowheader", { name: "tools/call/echo" })).toBeVisible();
    await expect(page.getByRole("img", { name: "Latency by elapsed second" }).locator("canvas")).toBeVisible();
    await page.reload();
    await expect(status).toHaveText("running");
    expect(frames.some(f => f.type === "snapshot" && (f.open_interval?.calls > 0 || f.series.length > 0))).toBe(true);
    await page.getByRole("button", { name: "Stop run" }).click();
    await expect(status).toHaveText("stopped");
    const last = frames.at(-1);
    expect(last.sessions).toBe(0);
    expect(last.in_flight).toBe(0);
    expect(last.calls.count).toBeGreaterThan(0);
    expect(last.calls.failed).toBe(0);
    const applied = { users: 3, threads: 1, duration_ms: 21600000, bucket_secs: 12, wait_ms: 400, timeout_ms: 2500 };
    expect(last.run).toMatchObject(applied);
    expect(last.open_interval).toBeUndefined();
    expect(last.series.reduce((count, p) => count + p.calls, 0)).toBe(last.calls.count);
    expect(last.operations.initialize.count).toBe(3);
    expect(JSON.stringify(frames)).not.toContain("PRIVATE_BROWSER_PAYLOAD");

    const file = join(report, "index.html");
    const exported = await request.get(`${url}/api/v1/runs/local/export`);
    const firstReport = await exported.body();
    expect(JSON.parse(await readFile(join(report, "run.json"), "utf8"))).toMatchObject(applied);
    await page.getByRole("button", { name: "Configure next run" }).click();
    await expect(status).toHaveText("ready");
    await expect(users).toHaveValue("3");
    await expect(threads).toHaveValue("1");
    await users.press("Home");
    await threads.press("ArrowRight");
    await expect(unit).toHaveValue("3600000");
    await unit.selectOption({ label: "seconds" });
    await duration.fill("1.5");
    await expect(wait).toHaveValue("400");
    await wait.fill("0");
    await page.getByRole("button", { name: "Start run" }).click();
    await expect(status).toHaveText("completed");
    const second = frames.at(-1);
    expect(second.run).toMatchObject({ ...applied, users: 1, threads: 2, duration_ms: 1500, bucket_secs: 1, wait_ms: 0 });
    await expect(page.getByText("No wait between calls", { exact: true })).toBeVisible();
    expect(second.operations.initialize.count).toBe(1);
    expect(second.calls.failed).toBe(0);
    const sibling = (await readdir(dirname(report))).find(name => name.startsWith("run-"));
    expect(sibling).toBeTruthy();
    expect(JSON.parse(await readFile(join(dirname(report), sibling, "run.json"), "utf8"))).toMatchObject(second.run);
    expect(await readFile(file)).toEqual(firstReport);
    const wsCount = frames.length;
    await context.setOffline(true);
    await page.goto(pathToFileURL(file).href);
    await expect(page.getByText("Offline report", { exact: true })).toBeVisible();
    await expect(page.getByRole("region", { name: "Live flock" })).toHaveCount(0);
    await bird.evaluate(img => img.decode());
    await expect(status).toHaveText("stopped");
    await expect(page.getByRole("button", { name: "Start run" })).toHaveCount(0);
    await expect(users).toHaveCount(0);
    await expect(page.getByText("3 concurrent users", { exact: true })).toBeVisible();
    await expect(page.getByRole("rowheader", { name: "tools/call/echo" })).toBeVisible();
    expect(frames.length).toBe(wsCount);
    expect(errors).toEqual([]);
  } finally {
    await Promise.all(children.reverse().map(stopChild));
  }
});

test("import a scenario, observe independent birds and reconnect a stale bird", async ({ page, request }, info) => {
  test.setTimeout(50000);
  const children = [];
  try {
    let forwardRoster = true;
    await page.routeWebSocket("**/api/v1/flock/live", client => {
      const server = client.connectToServer();
      server.onMessage(message => { if (forwardRoster) client.send(message); });
    });
    const report = info.outputPath("imported");
    const url = await launch(["demo","--web","--birds","1","--wait","375ms","--report",report], "Dashboard: ", children);
    await page.goto(url);
    const status = page.getByRole("status", {name:"Run status"});
    await expect(status).toHaveText("ready");
    const flock = page.getByRole("region", {name:"Live flock"});
    const counts = flock.locator(".flock-count");
    await expect(flock.getByRole("button", {name:"Bird bird-01"})).toBeVisible();
    await expect(flock).toContainText("This controller launched 1 local demo bird at startup.");
    const {home} = await (await request.get(`${url}/api/v1/flock`)).json();
    const startBird = () => launch(["bird","--id","wing-two","--home",home], "Bird wing-two connected", children);
    await startBird();
    const visitor = flock.getByRole("button", {name:"Bird wing-two"});
    await expect(visitor).toContainText("idle");
    await expect(counts).toContainText("2 reporting");
    const roster = async () => (await (await request.get(`${url}/api/v1/flock`)).json()).birds;
    await expect.poll(async () => (await roster()).find(b => b.id === "bird-01")?.requests ?? 0).toBeGreaterThan(0);
    expect(new Set((await roster()).map(b => b.pid)).size).toBe(2);
    await visitor.click();
    await expect(flock.getByLabel("Selected bird")).toContainText("0 requests");
    const heartbeat = flock.getByLabel("Selected bird").getByText(/^Heartbeat #\d+$/);
    await expect(heartbeat).toBeVisible();
    await expect(heartbeat).not.toHaveText(await heartbeat.innerText());
    forwardRoster = false;
    await expect(counts).toHaveText("No recent flock data", {timeout: 10000});
    await expect(visitor).toContainText("last known");
    forwardRoster = true;
    await expect(counts).toContainText("2 reporting");
    await stopChild(children.at(-1));
    await expect(visitor).toContainText("stale", {timeout:10000});
    await expect(counts).toHaveText(/1 reporting · \d+ running · 1 stale/);
    await startBird();
    await expect(visitor).toContainText("idle");
    await expect(flock.getByRole("button")).toHaveCount(2);

    const load = `${url}/api/v1/runs/local/workload`;
    const headers = {"X-Starling-Control":"1","Content-Type":"application/yaml"};
    const sample = await (await request.get(`${url}/api/v1/workload/example`)).text();
    const actualEndpoint = sample.match(/^  url: (http:\/\/127\.0\.0\.1:\d+\/mcp)$/m)[1];
    expect((await request.post(load,{data:sample})).status()).toBe(403);
    expect((await request.post(load,{headers,data:"x".repeat(65537)})).status()).toBe(413);
    const bad = await request.post(load,{headers,data:sample.replace(actualEndpoint,"https://other.invalid/private")});
    expect(bad.status()).toBe(422);
    expect(await bad.text()).not.toContain("private");
    const yaml = ("# " + ".".repeat(2500) + "\n" + sample)
      .replace(`url: ${actualEndpoint}`, `url: http://127.0.0.1:1/mcp\n  endpoints: { candidate: "${actualEndpoint}" }`)
      .replace("30s","3s").replace("How do I get started?", "PRIVATE_UPLOAD_QUESTION");
    await page.getByLabel("Load YAML").setInputFiles({name:"scenario.yaml",mimeType:"application/yaml",buffer:Buffer.from(yaml)});
    await expect(page.getByRole("heading",{name:"Example questions"})).toBeVisible();
    await expect(status).toHaveText("ready");
    await expect(page.getByRole("spinbutton",{name:"Duration",exact:true})).toHaveValue("3");
    await page.getByRole("combobox",{name:"Endpoint",exact:true}).selectOption("candidate");
    await page.getByRole("button",{name:"Start run"}).click();
    await expect(status).toHaveText("running");
    expect((await request.post(load,{headers,data:sample})).status()).toBe(409);
    await expect(status).toHaveText("completed",{timeout:9000});
    await expect(page.getByRole("rowheader",{name:"task/prompts/get/ask/ask-question"})).toBeVisible();
    expect((await readFile(join(report,"index.html"),"utf8"))).not.toContain("PRIVATE_UPLOAD_QUESTION");
    const complete=JSON.parse(await readFile(join(report,"summary.json"),"utf8"));
    expect(complete.run.endpoint).toBe("candidate");
    expect(complete.calls.failed).toBe(0);
    await expect.poll(async () => (await roster()).find(b => b.id === "bird-01")?.state, {timeout: 20000}).toBe("completed");
    expect(JSON.parse(await readFile(join(report,"birds","bird-01","run.json"),"utf8")).wait_ms).toBe(375);
  } finally { await Promise.all(children.reverse().map(stopChild)); }
});
