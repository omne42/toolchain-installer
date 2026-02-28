import test from "node:test";
import assert from "node:assert/strict";
import worker, { buildGitReleaseRedirect } from "./index.js";

test("buildGitReleaseRedirect accepts git release route", () => {
  const out = buildGitReleaseRedirect(
    "/toolchain/git/v2.48.1.windows.1/MinGit-2.48.1-busybox-64-bit.zip",
  );
  assert.equal(
    out,
    "https://github.com/git-for-windows/git/releases/download/v2.48.1.windows.1/MinGit-2.48.1-busybox-64-bit.zip",
  );
});

test("buildGitReleaseRedirect rejects non git route", () => {
  assert.equal(
    buildGitReleaseRedirect("/toolchain/gh/v2.48.1/gh_2.48.1_linux_amd64.tar.gz"),
    null,
  );
});

test("worker rejects non-CN country", async () => {
  const request = new Request("https://example.workers.dev/toolchain/git/v2.1.0/MinGit-2.1.0-64-bit.zip", {
    headers: { "cf-ipcountry": "US" },
  });
  const resp = await worker.fetch(request);
  assert.equal(resp.status, 403);
});

test("worker allows CN and redirects", async () => {
  const request = new Request("https://example.workers.dev/toolchain/git/v2.1.0/MinGit-2.1.0-64-bit.zip", {
    headers: { "cf-ipcountry": "CN" },
  });
  const resp = await worker.fetch(request);
  assert.equal(resp.status, 302);
  assert.match(
    resp.headers.get("location") || "",
    /^https:\/\/github\.com\/git-for-windows\/git\/releases\/download\//,
  );
});
