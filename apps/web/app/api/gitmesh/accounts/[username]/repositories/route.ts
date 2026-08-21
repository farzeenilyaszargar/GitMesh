import { NextRequest, NextResponse } from "next/server";

import {
  daemonJson,
  readJsonBody,
  requestDaemon,
  requireMutationAuth,
  safeAccountToken,
  safeToken
} from "../../../backend";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

type RouteContext = {
  params: Promise<{ username: string }>;
};

export async function POST(request: NextRequest, context: RouteContext) {
  const unauthorized = requireMutationAuth(request);
  if (unauthorized) {
    return unauthorized;
  }
  const { username } = await context.params;
  const body = await readJsonBody(request);
  const name = stringField(body.name);
  const repoId = stringField(body.repoId);
  const visibility = stringField(body.visibility) ?? "private";

  if (
    !safeAccountToken(username) ||
    !name ||
    !safeAccountToken(name) ||
    !repoId ||
    !safeToken(repoId) ||
    !["public", "private"].includes(visibility)
  ) {
    return NextResponse.json(
      { ok: false, error: "username, name, repoId, and visibility are invalid" },
      { status: 400 }
    );
  }

  return daemonJson(
    await requestDaemon(`REPO_REGISTER ${username} ${name} ${repoId} ${visibility}`, {
      admin: true
    }),
    201
  );
}

function stringField(value: unknown) {
  return typeof value === "string" ? value : undefined;
}
