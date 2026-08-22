import { NextResponse } from "next/server";

import { networkPeersFromFields, requestDaemon } from "../backend";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET() {
  const [status, peerList] = await Promise.all([
    requestDaemon("NETWORK_STATUS"),
    requestDaemon("NETWORK_PEER_LIST")
  ]);

  return NextResponse.json(
    {
      ok: status.ok && peerList.ok,
      status,
      peers: peerList.ok ? networkPeersFromFields(peerList.fields) : [],
      error: status.error ?? peerList.error
    },
    { status: status.ok && peerList.ok ? 200 : 503 }
  );
}
