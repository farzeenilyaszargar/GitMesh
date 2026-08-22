import { NextRequest, NextResponse } from "next/server";

import { parseNumberList, readJsonBody, requestDaemon, safeDaemonText } from "../backend";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET() {
  return networkRepairProof();
}

export async function POST(request: NextRequest) {
  const body = await readJsonBody(request);
  const payload = typeof body.payload === "string" ? body.payload.trim() : "";

  if (payload && !safeDaemonText(payload)) {
    return NextResponse.json(
      { ok: false, error: "payload must be 4096 characters or fewer and cannot contain newlines" },
      { status: 400 }
    );
  }

  return networkRepairProof(payload);
}

async function networkRepairProof(payload = "") {
  const command = payload ? `NETWORK_REPAIR_PROOF ${payload}` : "NETWORK_REPAIR_PROOF";
  const response = await requestDaemon(command);

  return NextResponse.json(
    {
      ...response,
      proof: response.ok
        ? {
            oid: response.fields.oid,
            recoveredExactly: response.fields.recovered_exactly === "true",
            repairedShards: parseNumberList(response.fields.repaired_shards),
            originalPeer: response.fields.original_peer,
            replacementPeer: response.fields.replacement_peer,
            providers: response.fields.providers ? Number(response.fields.providers) : 0,
            verifiedAfterRepair: response.fields.verified_after_repair
              ? Number(response.fields.verified_after_repair)
              : 0,
            durabilitySatisfied: response.fields.durability_satisfied === "true"
          }
        : null
    },
    { status: response.ok ? 200 : 503 }
  );
}
