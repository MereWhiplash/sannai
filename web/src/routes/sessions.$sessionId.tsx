import { useQuery } from '@tanstack/react-query'
import { createFileRoute, Link } from '@tanstack/react-router'
import { sessionQueryOptions, sessionEventsQueryOptions } from '~/utils/queries'
import { EventTimeline } from '~/components/EventTimeline'

export const Route = createFileRoute('/sessions/$sessionId')({
  loader: async ({ context, params }) => {
    await Promise.allSettled([
      context.queryClient.ensureQueryData(sessionQueryOptions(params.sessionId)),
      context.queryClient.ensureQueryData(sessionEventsQueryOptions(params.sessionId)),
    ])
  },
  component: SessionDetail,
})

function SessionDetail() {
  const { sessionId } = Route.useParams()
  const { data: session } = useQuery(sessionQueryOptions(sessionId))
  const { data: events = [] } = useQuery(sessionEventsQueryOptions(sessionId))

  if (!session) {
    return (
      <div className="p-6">
        <p className="text-gray-500">Loading session...</p>
      </div>
    )
  }

  return (
    <div className="p-6 max-w-5xl mx-auto space-y-6">
      <div className="flex items-center gap-2 text-sm text-gray-500">
        <Link to="/sessions" className="hover:underline">
          Sessions
        </Link>
        <span>/</span>
        <span className="font-mono">{sessionId.slice(0, 8)}</span>
      </div>

      <div className="space-y-1">
        <h1 className="text-xl font-bold font-mono">
          {session.project_path || sessionId}
        </h1>
        <div className="flex items-center gap-4 text-sm text-gray-500 dark:text-gray-400">
          <span>{session.tool}</span>
          <span>{new Date(session.started_at).toLocaleString()}</span>
          {session.ended_at ? (
            <span>
              Ended {new Date(session.ended_at).toLocaleString()}
            </span>
          ) : (
            <span className="text-emerald-600 dark:text-emerald-400 font-medium">
              Active
            </span>
          )}
          <span>{session.event_count} events</span>
        </div>
      </div>

      <EventTimeline events={events} />
    </div>
  )
}
