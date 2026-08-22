import { NextRequest, NextResponse } from "next/server";

import { pullRequestsFromFields, requestDaemon, safeDaemonText } from "../backend";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET(request: NextRequest) {
  const repo = request.nextUrl.searchParams.get("repo") ?? "farzeen/gitmesh";
  if (!safeDaemonText(repo)) {
    return NextResponse.json({ ok: false, error: "invalid repo" }, { status: 400 });
  }
  const response = await requestDaemon(`PR_LIST ${repo}`);
  return NextResponse.json(
    response.ok
      ? { ...response, pullRequests: pullRequestsFromFields(response.fields) }
      : response,
    { status: response.ok ? 200 : 503 }
  );
}
