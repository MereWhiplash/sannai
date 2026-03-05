import { z } from 'zod'

const AGENT_BASE = 'http://localhost:9847'

export const SessionSchema = z.object({
  id: z.string(),
  tool: z.string(),
  project_path: z.string().nullable(),
  started_at: z.string(),
  ended_at: z.string().nullable(),
  event_count: z.number(),
})

export const EventSchema = z.object({
  id: z.number().nullable(),
  session_id: z.string(),
  event_type: z.string(),
  content: z.string().nullable(),
  context_files: z.any().nullable().optional(),
  timestamp: z.string(),
  metadata: z.any().nullable().optional(),
})

export const HealthSchema = z.object({
  status: z.string(),
  version: z.string(),
})

export type Session = z.infer<typeof SessionSchema>
export type SessionEvent = z.infer<typeof EventSchema>
export type Health = z.infer<typeof HealthSchema>

async function agentFetch<T>(path: string, schema: z.ZodType<T>): Promise<T> {
  const res = await fetch(`${AGENT_BASE}${path}`)
  if (!res.ok) {
    throw new Error(`Agent API error: ${res.status} ${res.statusText}`)
  }
  const data = await res.json()
  return schema.parse(data)
}

export function fetchHealth() {
  return agentFetch('/health', HealthSchema)
}

export function fetchSessions(limit = 50, offset = 0) {
  return agentFetch(`/sessions?limit=${limit}&offset=${offset}`, z.array(SessionSchema))
}

export function fetchSession(id: string) {
  return agentFetch(`/sessions/${id}`, SessionSchema)
}

export function fetchSessionEvents(id: string) {
  return agentFetch(`/sessions/${id}/events`, z.array(EventSchema))
}
