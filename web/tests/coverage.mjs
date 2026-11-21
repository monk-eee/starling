import { execFileSync } from "node:child_process";
import { cpSync, globSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import coverage from "istanbul-lib-coverage";
import instrument from "istanbul-lib-instrument";
import report from "istanbul-lib-report";
import reports from "istanbul-reports";
import { build, normalizePath } from "vite";

const root = fileURLToPath(new URL("../../", import.meta.url));
const web = join(root, "web");
const output = process.argv[2];
if (!output) throw new Error("Run the dashboard collector via tests/coverage.py dashboard");
const counters = coverage.createCoverageMap({});
const modules = new Map();
for (const name of globSync("src/**/*.{ts,tsx}", { cwd: web }).filter(p => !p.endsWith(".d.ts"))) {
  const file = resolve(web, name);
  const id = relative(root, file).split(sep).join("/");
  const parser = instrument.createInstrumenter({ esModules: true, parserPlugins: ["typescript", "jsx"] });
  modules.set(normalizePath(file), parser.instrumentSync(readFileSync(file, "utf8"), id));
  counters.addFileCoverage(parser.lastFileCoverage());
}

// The CLI embeds the HTML at compile time. Never replace the dashboard or binary a developer is running.
const project = mkdtempSync(join(tmpdir(), "starling-dashboard-coverage-"));
try {
  for (const name of ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "build.rs", "src",
    "proto/flock.proto", "examples/local.yaml", "examples/local.toml"]) {
    mkdirSync(dirname(join(project, name)), { recursive: true });
    cpSync(join(root, name), join(project, name), { recursive: true });
  }
  await build({
    root: web,
    plugins: [{
      name: "starling-source-coverage", enforce: "pre",
      transform(_code, id) { return modules.get(id); },
    }],
    build: { outDir: join(project, "web", "dist"), emptyOutDir: false },
  });
  const captures = join(project, "captures");
  mkdirSync(captures);
  const env = { ...process.env, CARGO_TARGET_DIR: join(root, "target", "dashboard-coverage-build"),
    STARLING_COVERAGE_CAPTURES: captures };
  execFileSync("cargo", ["build", "--locked"], { cwd: project, env, stdio: "inherit" });
  execFileSync(process.execPath, [fileURLToPath(import.meta.resolve("@playwright/test/cli")),
    "test", "tests", "--output", join(env.CARGO_TARGET_DIR, "test-results")], { cwd: web, env, stdio: "inherit" });

  const schemes = new Set();
  for (const file of readdirSync(captures)) {
    const { scheme, hits } = JSON.parse(readFileSync(join(captures, file), "utf8"));
    schemes.add(scheme);
    for (const id of Object.keys(hits)) {
      if (!counters.files().includes(id)) throw new Error(`Unexpected dashboard source: ${id}`);
    }
    counters.merge(hits);
  }
  if (!schemes.has("http:") || !schemes.has("file:")) throw new Error("Missing live or offline dashboard coverage");
  const context = report.createContext({ dir: output, coverageMap: counters,
    sourceFinder: id => readFileSync(resolve(root, id), "utf8") });
  reports.create("lcovonly", { projectRoot: root }).execute(context);
  reports.create("html", { subdir: "html" }).execute(context);
} finally {
  rmSync(project, { recursive: true, force: true });
}
