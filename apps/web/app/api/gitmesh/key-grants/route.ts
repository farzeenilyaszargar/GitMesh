import { NextRequest, NextResponse } from "next/server";

import { requestDaemon } from "../backend";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

const DEFAULT_REPO_ID = "repo:farzeen/gitmesh";

export async function GET(request: NextRequest) {
  const repoId = request.nextUrl.searchParams.get("repoId") ?? DEFAULT_REPO_ID;
  const selector = request.nextUrl.searchParams.get("selector") ?? "latest";

  if (!isSafeToken(repoId) || !isSafeToken(selector)) {
    return NextResponse.json(
      { ok: false, error: "repoId and selector must be non-empty whitespace-free tokens" },
      { status: 400 }
    );
  }

  const response = await requestDaemon(`KEY_GRANT_LIST ${repoId} ${selector}`);
  return NextResponse.json(response, { status: response.ok ? 200 : 503 });
}

function isSafeToken(value: string) {
  return value.length > 0 && !/\s/.test(value);
}
