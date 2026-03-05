import { queryOptions } from '@tanstack/react-query'
import { fetchHealth, fetchSession, fetchSessionEvents, fetchSessions } from './api'

export const healthQueryOptions = () =>
  queryOptions({
    queryKey: ['health'],
    queryFn: fetchHealth,
    staleTime: 10_000,
  })

export const sessionsQueryOptions = (limit = 50, offset = 0) =>
  queryOptions({
    queryKey: ['sessions', { limit, offset }],
    queryFn: () => fetchSessions(limit, offset),
    staleTime: 5_000,
  })

export const sessionQueryOptions = (id: string) =>
  queryOptions({
    queryKey: ['session', id],
    queryFn: () => fetchSession(id),
    staleTime: 5_000,
  })

export const sessionEventsQueryOptions = (id: string) =>
  queryOptions({
    queryKey: ['session', id, 'events'],
    queryFn: () => fetchSessionEvents(id),
    staleTime: 5_000,
  })
