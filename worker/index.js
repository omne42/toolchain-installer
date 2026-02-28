const GIT_RELEASE_HOST = "https://github.com/git-for-windows/git/releases/download";

export function buildGitReleaseRedirect(pathname) {
  const match = pathname.match(/^\/toolchain\/git\/([^/]+)\/([^/]+)$/);
  if (!match) {
    return null;
  }
  const tag = decodeURIComponent(match[1] || "");
  const asset = decodeURIComponent(match[2] || "");
  if (!isValidTag(tag) || !isValidAsset(asset)) {
    return null;
  }
  return `${GIT_RELEASE_HOST}/${encodeURIComponent(tag)}/${encodeURIComponent(asset)}`;
}

function isValidTag(tag) {
  return /^v?[0-9]+\.[0-9]+\.[0-9]+(?:\.[A-Za-z0-9._-]+)?$/.test(tag);
}

function isValidAsset(asset) {
  return /^MinGit-[A-Za-z0-9._-]+\.zip$/.test(asset);
}

function getCountry(request) {
  const headerCountry = request.headers.get("cf-ipcountry");
  if (headerCountry && headerCountry.trim()) {
    return headerCountry.trim().toUpperCase();
  }
  const cfCountry = request.cf && typeof request.cf.country === "string" ? request.cf.country : "";
  return cfCountry.trim().toUpperCase();
}

export default {
  async fetch(request) {
    if (request.method !== "GET" && request.method !== "HEAD") {
      return new Response("method_not_allowed", { status: 405 });
    }

    const country = getCountry(request);
    if (country !== "CN") {
      return new Response("forbidden_country", { status: 403 });
    }

    const url = new URL(request.url);
    const redirectUrl = buildGitReleaseRedirect(url.pathname);
    if (!redirectUrl) {
      return new Response("forbidden_route", { status: 403 });
    }

    return Response.redirect(redirectUrl, 302);
  },
};
