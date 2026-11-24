import { expect, test } from "@playwright/test";
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = fileURLToPath(new URL("../../", import.meta.url));
const binary = resolve(root, process.env.CARGO_TARGET_DIR ?? "target", "debug",
  process.platform === "win32" ? "starling.exe" : "starling");
if (process.env.STARLING_BROWSER_CHANNEL) test.use({ channel: process.env.STARLING_BROWSER_CHANNEL });

async function captureCoverage(page, info, label) {
  const dir = process.env.STARLING_COVERAGE_CAPTURES;
  if (!dir) return;
  const hits = await page.evaluate(() => globalThis.__coverage__);
  if (!hits || !Object.keys(hits).length) throw new Error(`No dashboard coverage at ${label}`);
  await writeFile(join(dir, `${info.testId}-${label}.json`), JSON.stringify({ scheme: new URL(page.url()).protocol, hits }));
}

test.afterEach(async ({ page }, info) => {
  if (info.status === "passed") await captureCoverage(page, info, "final");
});

function launch(args, prefix, children) {
  const child = spawn(binary, args, { cwd: root, stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env, STARLING_TELEMETRY: "none" } });
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
    expect((await request.post(start, { headers, data: { protocol: "not-a-revision" } })).status()).toBe(400);
    for (const data of [{ users: 9 }, { threads: 0 }, { duration_ms: 999 }, { duration_ms: 604800001 }, { wait_ms: 60001 }, { timeout_ms: 15001 }, { report_formats: [] }]) {
      expect((await request.post(start, { headers, data })).status()).toBe(422);
    }
    const users = page.getByRole("slider", { name: "Concurrent users" });
    const threads = page.getByRole("slider", { name: "Worker threads" });
    const protocol = page.getByRole("combobox", { name: "MCP protocol revision" });
    await expect(protocol).toHaveValue("2025-11-25");
    await protocol.selectOption("2025-06-18");
    const checks = page.getByRole("region", {name: "MCP conformance report"});
    expect((await request.post(`${url}/api/v1/runs/local/check`, {data:{all_protocols:true}})).status()).toBe(403);
    await page.getByRole("button", {name: "Check revision", exact:true}).click();
    await expect(checks.getByRole("heading", {name:"2025-06-18: pass"})).toBeVisible();
    await page.getByRole("button", {name:"Check all revisions"}).click();
    await expect(checks.getByRole("heading", {name:"2026-07-28: pass"})).toBeVisible();
    const link = await checks.getByRole("link", {name:"Download conformance JSON"}).getAttribute("href");
    const evidence = JSON.parse(decodeURIComponent(link.split(",")[1]));
    expect(evidence.verdict).toBe("pass");
    expect(evidence.profiles).toHaveLength(4);
    expect(evidence.profiles[0].checks.find(c => c.rule === "catalog.resources").status).toBe("skip");
    await expect(status).toHaveText("ready");
    expect((await (await request.get(`${url}/api/v1/runs/local`)).json()).calls.count).toBe(0);
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
    expect((await request.post(`${url}/api/v1/runs/local/check`, {headers, data:{all_protocols:true}})).status()).toBe(409);
    await expect(threads).toHaveCount(0);
    await expect(protocol).toHaveCount(0);
    await expect.poll(() => frames.some(f => f.type === "tick" && f.open_interval?.calls > 0)).toBe(true);
    await expect(page.getByRole("rowheader", { name: "tools/call/echo" })).toBeVisible();
    await expect(page.getByRole("img", { name: "Latency by elapsed second" }).locator("canvas")).toBeVisible();
    await captureCoverage(page, info, "before-reload");
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
    const applied = { protocol: "2025-06-18", users: 3, threads: 1, duration_ms: 21600000, bucket_secs: 12, wait_ms: 400, timeout_ms: 2500 };
    expect(last.run).toMatchObject(applied);
    expect(last.protocols).toEqual(["2025-06-18"]);
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
    await expect(protocol).toHaveValue("2025-06-18");
    await protocol.selectOption("2026-07-28");
    await users.press("Home");
    await threads.press("ArrowRight");
    await expect(unit).toHaveValue("3600000");
    await unit.selectOption({ label: "seconds" });
    await duration.fill("1.5");
    await expect(wait).toHaveValue("400");
    await wait.fill("0");
    const formats = page.getByRole("group", { name: "Save report as" });
    await formats.getByLabel("HTML", {exact:true}).uncheck();
    await formats.getByLabel("CSV", {exact:true}).uncheck();
    await formats.getByLabel("YAML", {exact:true}).check();
    await page.getByRole("button", { name: "Start run" }).click();
    await expect(status).toHaveText("completed");
    const second = frames.at(-1);
    expect(second.run).toMatchObject({ ...applied, protocol: "2026-07-28", users: 1, threads: 2, duration_ms: 1500, bucket_secs: 1, wait_ms: 0 });
    await expect(page.getByText("No wait between calls", { exact: true })).toBeVisible();
    expect(second.operations["server/discover"].count).toBe(1);
    expect(second.protocols).toEqual(["2026-07-28"]);
    expect(second.calls.failed).toBe(0);
    const sibling = (await readdir(dirname(report))).find(name => name.startsWith("run-"));
    expect(sibling).toBeTruthy();
    expect(JSON.parse(await readFile(join(dirname(report), sibling, "run.json"), "utf8"))).toMatchObject(second.run);
    const secondDir = join(dirname(report), sibling);
    expect((await readdir(secondDir)).sort()).toEqual(["junit.xml","run.json","summary.json","summary.yaml","timeseries.json"]);
    await page.getByLabel("Download file").selectOption("junit.xml");
    const download = await page.getByRole("link",{name:"Export junit.xml"}).getAttribute("href");
    const xml = await request.get(new URL(download, url).href);
    expect(xml.headers()["content-type"]).toBe("application/xml");
    expect(await xml.body()).toEqual(await readFile(join(secondDir, "junit.xml")));
    expect((await request.get(`${url}/api/v1/runs/local/export?file=workload.yaml`)).status()).toBe(400);
    expect((await request.get(`${url}/api/v1/runs/local/export`)).status()).toBe(200);
    expect(await readFile(file)).toEqual(firstReport);
    const wsCount = frames.length;
    await captureCoverage(page, info, "before-offline");
    await context.setOffline(true);
    await page.goto(pathToFileURL(file).href);
    await expect(page.getByText("Offline report", { exact: true })).toBeVisible();
    await expect(page.getByRole("region", { name: "Live flock" })).toHaveCount(0);
    await bird.evaluate(img => img.decode());
    await expect(status).toHaveText("stopped");
    await expect(page.getByText("Requested MCP: 2025-06-18")).toBeVisible();
    await expect(page.getByText("Negotiated MCP: 2025-06-18")).toBeVisible();
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
    const url = await launch(["demo","--web","--birds","1","--protocol","2025-06-18","--wait","375ms","--format","json","--report",report], "Dashboard: ", children);
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
    expect((await request.post(load,{headers,data:sample.replace('protocol: "2025-11-25"', 'protocol: "2024-11-05"')})).status()).toBe(422);
    expect((await request.post(load,{data:sample})).status()).toBe(403);
    expect((await request.post(load,{headers,data:"x".repeat(65537)})).status()).toBe(413);
    const bad = await request.post(load,{headers,data:sample.replace(actualEndpoint,"https://other.invalid/mcp?private")});
    expect(bad.status()).toBe(422);
    expect(await bad.text()).not.toContain("private");
    const yaml = ("# " + ".".repeat(2500) + "\n" + sample)
      .replace(`url: ${actualEndpoint}`, `url: https://other.invalid/mcp\n  endpoints: { candidate: "${actualEndpoint}" }`)
      .replace('protocol: "2025-11-25"', 'protocol: "2025-03-26"')
      .replace("30s","3s").replace("How do I get started?", "PRIVATE_UPLOAD_QUESTION")
      + "\nreport:\n  formats: [html, json]\nassert:\n  p95: '< 15s'\n  min_requests: 1\n";
    const upload = page.getByLabel("Load YAML", {exact:true});
    await upload.setInputFiles({name:"broken.yaml",mimeType:"application/yaml",buffer:Buffer.from("run: [")});
    await expect(page.getByRole("alert")).toContainText("Could not load workload:");
    await expect(status).toHaveText("ready");
    await page.getByText("Paste YAML instead", {exact:true}).click();
    await page.getByLabel("YAML text", {exact:true}).fill(yaml.replace("Example questions","Paste preview"));
    await page.getByRole("button",{name:"Load pasted YAML"}).click();
    const loaded = page.getByRole("status",{name:"Loaded workload"});
    await expect(loaded).toContainText("Loaded pasted YAML: Paste preview.");
    await expect(page.getByLabel("YAML text",{exact:true})).toHaveValue("");
    const file = info.outputPath("scenario.yaml");
    await writeFile(file,yaml);
    const chooser = page.waitForEvent("filechooser");
    const accepted = page.waitForResponse(r => r.url() === load && r.request().method() === "POST");
    await upload.click();
    await (await chooser).setFiles(file);
    const receipt = await accepted;
    expect(receipt.status()).toBe(202);
    expect((await receipt.json()).generation).toBeGreaterThan(1);
    await expect(loaded).toContainText("Loaded scenario.yaml: Example questions.");
    await expect(upload).toHaveValue(/scenario\.yaml$/);
    expect((await (await request.get(`${url}/api/v1/runs/local`)).json()).calls.count).toBe(0);
    await expect(page.getByRole("heading",{name:"Example questions"})).toBeVisible();
    await expect(page.getByRole("alert")).toHaveCount(0);
    await expect(status).toHaveText("ready");
    await expect(page.getByRole("spinbutton",{name:"Duration",exact:true})).toHaveValue("3");
    await expect(page.getByRole("combobox",{name:"MCP protocol revision"})).toHaveValue("2025-03-26");
    await expect(page.getByRole("checkbox",{name:"HTML",exact:true})).toBeChecked();
    await expect(page.getByRole("checkbox",{name:"CSV",exact:true})).not.toBeChecked();
    const gate = page.getByRole("region", {name: "Performance gate"});
    await expect(gate).toContainText("min_requests >= 1");
    await page.getByRole("combobox",{name:"Endpoint",exact:true}).selectOption("candidate");
    await page.getByRole("button",{name:"Start run"}).click();
    await expect(status).toHaveText("running");
    expect((await request.post(load,{headers,data:sample})).status()).toBe(409);
    await expect(status).toHaveText("completed",{timeout:9000});
    await expect(page.getByRole("rowheader",{name:"task/prompts/get/ask/ask-question"})).toBeVisible();
    expect((await readFile(join(report,"index.html"),"utf8"))).not.toContain("PRIVATE_UPLOAD_QUESTION");
    const complete=JSON.parse(await readFile(join(report,"summary.json"),"utf8"));
    expect(complete.run.endpoint).toBe("candidate");
    expect(complete.run.protocol).toBe("2025-03-26");
    expect(complete.protocols).toEqual(["2025-03-26"]);
    expect(complete.calls.failed).toBe(0);
    await expect(gate.getByRole("heading")).toHaveText("Performance gate: passed");
    expect(complete.acceptance.status).toBe("passed");
    await expect.poll(async () => (await roster()).find(b => b.id === "bird-01")?.state, {timeout: 20000}).toBe("completed");
    const childReport = JSON.parse(await readFile(join(report,"birds","bird-01","summary.json"),"utf8"));
    expect(childReport.run).toMatchObject({ wait_ms: 375, protocol: "2025-06-18", report_formats:["json"] });
    expect((await readdir(join(report,"birds","bird-01"))).sort()).toEqual(["run.json","summary.json","timeseries.json"]);
    expect(childReport.protocols).toEqual(["2025-06-18"]);

    await page.getByRole("button", {name:"Configure next run"}).click();
    await expect(status).toHaveText("ready");
    await page.getByRole("spinbutton",{name:"Duration",exact:true}).fill("60");
    await page.getByRole("button",{name:"Start run"}).click();
    await expect.poll(async () => (await (await request.get(`${url}/api/v1/runs/local`)).json()).calls.count).toBeGreaterThan(0);
    await page.getByRole("button",{name:"Stop run"}).click();
    await expect(status).toHaveText("failed");
    await expect(gate.getByRole("heading")).toHaveText("Performance gate: incomplete");
    await expect(gate.getByRole("row", { name: /min_requests/ })).toContainText("Not evaluated");

    await captureCoverage(page, info, "before-gated-report");
    await page.context().setOffline(true);
    await page.goto(pathToFileURL(join(report, "index.html")).href);
    await expect(gate.getByRole("heading")).toHaveText("Performance gate: passed");
    await expect(gate.getByRole("row", { name: /min_requests/ })).toContainText(`${complete.calls.count} requests`);
  } finally { await Promise.all(children.reverse().map(stopChild)); }
});
test("show a once-per-case assertion suite in the live and offline dashboard", async ({page,request},info) => {
  const children=[];
  try {
    const target=await launch(["fixture","--listen","127.0.0.1:0"],"Fixture: ",children);
    const config=info.outputPath("cases.yaml"), report=info.outputPath("native");
    await mkdir(dirname(config),{recursive:true});
    const tasks=Array.from({length:25},(_,i)=>({name:`case-${i+1}`,title:`Question ${i+1}`,tool:"echo",weight:1,
      arguments:{text:"PRIVATE_BROWSER_EVIDENCE ready"},assert_match:{label:"Response contains ready",path:"/content",first:5,
        all:[{path:"/text",matches:[i===24?"PRIVATE_ABSENT":"ready"]}]}}));
    await writeFile(config,JSON.stringify({run:{name:"Fixture cases",mode:"once",seed:42,duration:"45s"},
      target:{url:target,protocol:"2025-06-18"},load:{users:1,timeout:"3s",wait:0},
      report:{formats:["html","yaml"]},tasks}));
    const url=await launch(["run","--web","--listen","127.0.0.1:0","--config",config,"--report",report],"Dashboard: ",children);
    await page.goto(url);
    const cases=page.getByRole("region",{name:"Case assertions"});
    await expect(cases.getByRole("heading")).toHaveText("Case assertions: 0/25 passed");
    await expect(cases.getByRole("row")).toHaveCount(26);
    await expect(cases.getByText("Pending",{exact:true})).toHaveCount(25);
    await page.getByRole("button",{name:"Start run",exact:true}).click();
    await expect(cases.getByRole("heading")).toHaveText("Case assertions: 24/25 passed");
    await expect(page.getByRole("status",{name:"Run status"})).toHaveText("failed");
    await expect(cases.getByRole("row").last()).toContainText("Failed");
    await expect(cases.getByText("Last match: #1",{exact:true})).toHaveCount(24);
    await page.getByLabel("Download file").selectOption("summary.yaml");
    const link=await page.getByRole("link",{name:"Export summary.yaml"}).getAttribute("href");
    const response=await request.get(new URL(link,url).href);
    expect(response.headers()["content-type"]).toBe("application/yaml");
    expect(await response.body()).toEqual(await readFile(join(report,"summary.yaml")));
    await page.context().setOffline(true);
    await page.goto(pathToFileURL(join(report,"index.html")).href);
    await expect(cases.getByRole("heading")).toHaveText("Case assertions: 24/25 passed");
    await expect(cases.getByRole("row").last()).toContainText("Question 25");
    expect(await readFile(join(report,"index.html"),"utf8")).not.toContain("PRIVATE_BROWSER_EVIDENCE");
  } finally { await Promise.all(children.reverse().map(stopChild)); }
});

test("rejected controls keep the current workload and a timed-out target cannot pass", async ({page,request},info) => {
  const children = [];
  const silent = createServer(() => {}); // Accept loopback requests but never finish initialization.
  await new Promise(resolve => silent.listen(0, "127.0.0.1", resolve));
  try {
    const report = info.outputPath("report"), config = info.outputPath("timeout.yaml");
    await mkdir(dirname(config), {recursive:true});
    await writeFile(config, JSON.stringify({
      run:{name:"Timeout case", mode:"once", seed:1, duration:"10s"},
      target:{url:`http://127.0.0.1:${silent.address().port}/mcp`, protocol:"2025-06-18"},
      load:{users:1, timeout:"500ms"}, tasks:[{tool:"status",weight:1}],
      assert:{min_requests:1,p95:"< 1s"},
    }));
    const url = await launch(["run","--web","--listen","127.0.0.1:0","--config",config,"--report",report],
      "Dashboard: ", children);
    const api = `${url}/api/v1/runs/local`;
    await page.goto(url);
    const status = page.getByRole("status",{name:"Run status"});
    const upload = page.getByLabel("Load YAML",{exact:true});
    const yamlError = page.getByRole("region",{name:"YAML workload"}).getByRole("alert");
    await expect(status).toHaveText("ready");
    let uploads = 0;
    page.on("request", req => { if (req.url() === `${api}/workload`) uploads++; });
    await upload.setInputFiles({ name:"oversize.yaml", mimeType:"application/yaml", buffer:Buffer.alloc(65537, 32) });
    await expect(yamlError).toContainText("64 KiB or smaller");
    await upload.setInputFiles({ name:"bad-encoding.yaml", mimeType:"application/yaml", buffer:Buffer.from([0xff]) });
    await expect(yamlError).toContainText("Could not read the file as UTF-8");
    expect(uploads).toBe(0);

    await page.route(`${api}/workload`, route => route.fulfill({status:409}), {times:1});
    await upload.setInputFiles(config);
    await expect(yamlError).toContainText("A run is already in progress");
    await expect(page.getByRole("status",{name:"Loaded workload"})).toContainText("Current workload: Timeout case");
    const retry = page.waitForEvent("filechooser");
    await upload.click();
    await (await retry).setFiles(config);
    await expect(page.getByRole("status",{name:"Loaded workload"})).toContainText("Loaded timeout.yaml: Timeout case");
    await expect(yamlError).toHaveCount(0);

    const check = page.getByRole("button",{name:"Check revision",exact:true});
    const alert = page.getByRole("alert");
    await page.route(`${api}/check`, route => route.fulfill({status:503}), {times:1});
    await check.click();
    await expect(alert).toHaveText("Could not check conformance: HTTP 503.");
    await check.click();
    const evidence = page.getByRole("region",{name:"MCP conformance report"});
    await expect(evidence.getByRole("heading",{name:"MCP conformance: inconclusive"})).toBeVisible();
    await expect(evidence).toContainText("Negotiated: none");
    await expect(alert).toHaveCount(0);
    expect((await (await request.get(api)).json()).calls.count).toBe(0);

    const start = page.getByRole("button",{name:"Start run",exact:true});
    await page.route(`${api}/start`, route => route.continue({postData:JSON.stringify({users:0})}), {times:1});
    await start.click();
    await expect(alert).toContainText("Could not start this run: load.users must be 1-8");
    await expect(status).toHaveText("ready");
    await start.click();
    await expect(status).toHaveText("failed");
    await expect(page.getByText("No negotiated revision recorded",{exact:false})).toBeVisible();
    const gate = page.getByRole("region",{name:"Performance gate"});
    await expect(gate.getByRole("heading")).toHaveText("Performance gate: incomplete");
    await expect(gate.getByRole("row",{name:/p95/})).toContainText("No samples");
    await expect(page.getByRole("region",{name:"Case assertions"})).toContainText("Not run");
    const saved = JSON.parse(await readFile(join(report,"summary.json"),"utf8"));
    expect(saved.calls.count).toBe(0);
    const next = page.getByRole("button",{name:"Configure next run"});
    await page.route(`${api}/next`, route => route.fulfill({status:503}), {times:1});
    await next.click();
    await expect(page.getByText("Could not prepare the next run: HTTP 503.",{exact:true})).toBeVisible();
    await expect(status).toHaveText("failed");
    await next.click();
    await expect(status).toHaveText("ready");
    await expect(alert).toHaveCount(0);
  } finally {
    await Promise.all(children.reverse().map(stopChild));
    silent.closeAllConnections();
    await new Promise(resolve => silent.close(resolve));
  }
});

test("a gap requests a snapshot and an invalid feed reconnects without displaying bad metrics", async ({page,request},info) => {
  const children = [];
  try {
    const url = await launch(["demo","--web","--report",info.outputPath("report")],"Dashboard: ",children);
    const frame = await (await request.get(`${url}/api/v1/runs/local`)).json();
    const connections = [];
    let resyncs = 0;
    await page.routeWebSocket("**/api/v1/runs/local/live", ws => {
      connections.push(ws);
      ws.onMessage(text => { if (text === "snapshot") resyncs++; });
      if (connections.length === 1) ws.send(JSON.stringify({...frame, type:"snapshot", generation:1}));
    });
    await page.goto(url);
    const connection = page.getByRole("navigation").locator("small");
    const title = page.getByRole("heading",{level:1});
    const start = page.getByRole("button",{name:"Start run",exact:true});
    await expect(connection).toHaveText("Connected");
    const gap = {...frame, generation:1, type:"tick", sequence:frame.sequence+2, run:{...frame.run, name:"Skipped state"}};
    connections[0].send(JSON.stringify(gap));
    await expect.poll(() => resyncs).toBe(1);
    await expect(title).toHaveText(frame.run.name);
    connections[0].send(JSON.stringify({...gap, type:"snapshot",run:{...frame.run,name:"Restored snapshot"}}));
    await expect(title).toHaveText("Restored snapshot");

    connections[0].send(JSON.stringify({schema_version:99}));
    await expect(page.getByRole("alert")).toContainText("incompatible metrics frame");
    await expect(connection).toHaveText("Reconnecting");
    await expect(start).toBeDisabled();
    await expect.poll(() => connections.length).toBe(2);
    connections[1].send(JSON.stringify({...frame, type:"snapshot", generation:2}));
    await expect(connection).toHaveText("Connected");
    await expect(title).toHaveText(frame.run.name);
    await expect(start).toBeEnabled();
    await expect(page.getByRole("alert")).toHaveCount(0);
  } finally { await Promise.all(children.reverse().map(stopChild)); }
});
