import { NextRequest, NextResponse } from "next/server";

import {
  daemonJson,
  encodeLabelArg,
  encodeTextArg,
  issuesFromFields,
  readJsonBody,
  requestDaemon,
  requireMutationAuth,
  safeAccountToken,
  safeDaemonText,
  safeLabelList
} from "../backend";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET(request: NextRequest) {
  const repo = request.nextUrl.searchParams.get("repo") ?? "farzeen/gitmesh";
  if (!safeDaemonText(repo)) {
    return NextResponse.json({ ok: false, error: "invalid repo" }, { status: 400 });
  }
  const response = await requestDaemon(`ISSUE_LIST ${repo}`);
  return NextResponse.json(
    response.ok ? { ...response, issues: issuesFromFields(response.fields) } : response,
    { status: response.ok ? 200 : 503 }
  );
}

export async function POST(request: NextRequest) {
  const unauthorized = requireMutationAuth(request);
  if (unauthorized) {
    return unauthorized;
  }
  const body = await readJsonBody(request);
  const repo = stringField(body.repo) ?? "farzeen/gitmesh";
  const actor = stringField(body.actor) ?? "farzeen";
  const title = stringField(body.title);
  const issueBody = stringField(body.body);

  if (
    !safeDaemonText(repo) ||
    !safeAccountToken(actor) ||
    !title ||
    !safeDaemonText(title) ||
    !safeDaemonText(issueBody ?? "") ||
    !safeLabelList(body.labels)
  ) {
    return NextResponse.json(
      { ok: false, error: "repo, actor, title, body, or labels are invalid" },
      { status: 400 }
    );
  }

  return daemonJson(
    await requestDaemon(
      `ISSUE_OPEN ${repo} ${actor} ${encodeTextArg(title)} ${encodeTextArg(issueBody)} ${encodeLabelArg(body.labels)}`,
      { admin: true }
    ),
    201
  );
}

function stringField(value: unknown) {
  return typeof value === "string" ? value : undefined;
}
