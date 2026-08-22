import { NextRequest, NextResponse } from "next/server";

import {
  daemonJson,
  encodeLabelArg,
  encodeTextArg,
  pullRequestsFromFields,
  readJsonBody,
  requestDaemon,
  requireMutationAuth,
  safeAccountToken,
  safeDaemonText,
  safeLabelList,
  safeToken
} from "../backend";

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

export async function POST(request: NextRequest) {
  const unauthorized = requireMutationAuth(request);
  if (unauthorized) {
    return unauthorized;
  }
  const body = await readJsonBody(request);
  const repo = stringField(body.repo) ?? "farzeen/gitmesh";
  const actor = stringField(body.actor) ?? "farzeen";
  const sourceRef = stringField(body.sourceRef);
  const targetRef = stringField(body.targetRef) ?? "refs/heads/main";
  const title = stringField(body.title);
  const pullBody = stringField(body.body);

  if (
    !safeDaemonText(repo) ||
    !safeAccountToken(actor) ||
    !sourceRef ||
    !safeToken(sourceRef) ||
    !safeToken(targetRef) ||
    !title ||
    !safeDaemonText(title) ||
    !safeDaemonText(pullBody ?? "") ||
    !safeLabelList(body.labels)
  ) {
    return NextResponse.json(
      { ok: false, error: "repo, actor, refs, title, body, or labels are invalid" },
      { status: 400 }
    );
  }

  return daemonJson(
    await requestDaemon(
      `PR_OPEN ${repo} ${actor} ${sourceRef} ${targetRef} ${encodeTextArg(title)} ${encodeTextArg(pullBody)} ${encodeLabelArg(body.labels)}`,
      { admin: true }
    ),
    201
  );
}

function stringField(value: unknown) {
  return typeof value === "string" ? value : undefined;
}
