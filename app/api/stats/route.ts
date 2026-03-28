import { NextRequest, NextResponse } from "next/server";
import { prisma } from "@/lib/prisma";
import { verifyToken, extractToken } from "@/lib/auth";
import { withCache, cacheInvalidatePrefix } from "@/lib/cache";

// Stats are cached per-user for 60 seconds to avoid hammering the DB on every page load
const STATS_TTL_MS = 60_000;

export async function GET(request: NextRequest) {
  const token = extractToken(request.headers.get("authorization"));
  if (!token) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  const payload = verifyToken(token);
  if (!payload) {
    return NextResponse.json(
      { error: "Invalid or expired token" },
      { status: 401 },
    );
  }

  const userId = payload.userId;

  try {
    const stats = await withCache(`stats:${payload.userId}`, STATS_TTL_MS, () =>
      fetchStats(payload.userId),
    );

    return NextResponse.json(stats);
  } catch (error) {
    console.error("Stats error:", error);
    return NextResponse.json(
      { error: "Internal server error" },
      { status: 500 },
    );
  }
}

async function fetchStats(userId: string) {
  /**
   * Bundle all aggregate queries into a single round-trip using Promise.all().
   * This mitigates latency by executing all queries in parallel.
   * Uses Prisma's native count() and aggregate() with _sum for optimal performance.
   */

  // Get circle IDs where user is organizer or member (needed for member count)
  const userCircleIds = await prisma.circleMember.findMany({
    where: { userId, status: "ACTIVE" },
    select: { circleId: true },
  });

  const circleIds = userCircleIds.map(
    (cm: { circleId: string }) => cm.circleId,
  );

  // Also get circles where user is organizer
  const organizedCircles = await prisma.circle.findMany({
    where: { organizerId: userId, status: "ACTIVE" },
    select: { id: true },
  });

  const organizedCircleIds = organizedCircles.map((c: { id: string }) => c.id);

  // Combine all circle IDs (deduplicated)
  const allCircleIds = [...new Set([...circleIds, ...organizedCircleIds])];

  // Bundle all aggregate queries in parallel
  const [
    activeCirclesCount,
    contributionAggregates,
    totalMembers,
    withdrawalAggregates,
  ] = await Promise.all([
    // Count active circles where user is organizer or member
    prisma.circle.count({
      where: {
        status: "ACTIVE",
        OR: [
          { organizerId: userId },
          { members: { some: { userId, status: "ACTIVE" } } },
        ],
      },
    }),

    // Aggregate completed contributions: sum + count
    prisma.contribution.aggregate({
      _sum: { amount: true },
      _count: { _all: true },
      where: { userId, status: "COMPLETED" },
    }),

    // Count total active members across circles the user belongs to
    prisma.circleMember.count({
      where: {
        status: "ACTIVE",
        circleId: { in: allCircleIds },
      },
    }),

    // Aggregate completed withdrawals: sum
    prisma.withdrawal.aggregate({
      _sum: { amount: true },
      where: { userId, status: "COMPLETED" },
    }),
  ]);

  return {
    activeCircles: activeCirclesCount,
    totalContributed: contributionAggregates._sum.amount ?? 0,
    contributionCount: contributionAggregates._count._all,
    totalMembers,
    totalWithdrawn: withdrawalAggregates._sum.amount ?? 0,
  };
}

/**
 * Exported so contribution/withdrawal mutation routes can bust the cache
 * after a write: cacheInvalidatePrefix(`stats:${userId}`)
 */
export { cacheInvalidatePrefix };
