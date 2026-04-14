const DEFAULT_BUCKET_MS = 60_000
const DEFAULT_RETENTION_MS = 24 * 60 * 60_000

interface ClientRouteMetricEvent {
  at?: number
  clientVersion: string
  clientPlatform: string | null
  clientArch: string | null
  method: string
  route: string
  userId: string | null
}

export interface ClientRoutePersistedMetric {
  bucketStart: number
  clientVersion: string
  clientPlatform: string | null
  clientArch: string | null
  method: string
  route: string
  requests: number
  uniqueUserIds: string[]
  lastSeenAt: number
}

interface BucketEntry {
  clientVersion: string
  clientPlatform: string | null
  clientArch: string | null
  method: string
  route: string
  requests: number
  userIds: Set<string>
  lastSeenAt: number
}

function bucketKey(version: string, method: string, route: string): string {
  return `${version}|${method}|${route}`
}

class ClientAnalyticsCollector {
  private readonly buckets = new Map<number, Map<string, BucketEntry>>()
  private readonly pendingMetrics = new Map<string, ClientRoutePersistedMetric>()

  constructor(
    private readonly now: () => number = () => Date.now(),
    private readonly bucketMs: number = DEFAULT_BUCKET_MS,
    private readonly retentionMs: number = DEFAULT_RETENTION_MS,
  ) {}

  record(event: ClientRouteMetricEvent): void {
    const ts = event.at ?? this.now()
    const bucketStart = Math.floor(ts / this.bucketMs) * this.bucketMs
    const key = bucketKey(event.clientVersion, event.method, event.route)

    let bucket = this.buckets.get(bucketStart)
    if (!bucket) {
      bucket = new Map()
      this.buckets.set(bucketStart, bucket)
    }

    let entry = bucket.get(key)
    if (!entry) {
      entry = {
        clientVersion: event.clientVersion,
        clientPlatform: event.clientPlatform,
        clientArch: event.clientArch,
        method: event.method,
        route: event.route,
        requests: 0,
        userIds: new Set(),
        lastSeenAt: ts,
      }
      bucket.set(key, entry)
    }

    entry.requests++
    if (event.userId) {
      entry.userIds.add(event.userId)
    }
    entry.lastSeenAt = Math.max(entry.lastSeenAt, ts)

    // Also track in pending for persistence
    const persistKey = `${bucketStart}|${key}`
    const pending = this.pendingMetrics.get(persistKey)
    if (pending) {
      pending.requests++
      if (event.userId && !pending.uniqueUserIds.includes(event.userId)) {
        pending.uniqueUserIds.push(event.userId)
      }
      pending.lastSeenAt = Math.max(pending.lastSeenAt, ts)
    }
    else {
      this.pendingMetrics.set(persistKey, {
        bucketStart,
        clientVersion: event.clientVersion,
        clientPlatform: event.clientPlatform,
        clientArch: event.clientArch,
        method: event.method,
        route: event.route,
        requests: 1,
        uniqueUserIds: event.userId ? [event.userId] : [],
        lastSeenAt: ts,
      })
    }
  }

  drainPersistedMetrics(): { metrics: ClientRoutePersistedMetric[] } {
    const metrics = [...this.pendingMetrics.values()]
    this.pendingMetrics.clear()
    return { metrics }
  }

  requeuePersistedMetrics(metrics: ClientRoutePersistedMetric[]): void {
    for (const metric of metrics) {
      const key = `${metric.bucketStart}|${bucketKey(metric.clientVersion, metric.method, metric.route)}`
      const existing = this.pendingMetrics.get(key)
      if (existing) {
        existing.requests += metric.requests
        for (const uid of metric.uniqueUserIds) {
          if (!existing.uniqueUserIds.includes(uid)) {
            existing.uniqueUserIds.push(uid)
          }
        }
        existing.lastSeenAt = Math.max(existing.lastSeenAt, metric.lastSeenAt)
      }
      else {
        this.pendingMetrics.set(key, { ...metric })
      }
    }
  }

  prune(now?: number): void {
    const cutoff = (now ?? this.now()) - this.retentionMs
    for (const [bucketStart] of this.buckets) {
      if (bucketStart < cutoff) {
        this.buckets.delete(bucketStart)
      }
    }
  }
}

export const clientAnalyticsCollector = new ClientAnalyticsCollector()
