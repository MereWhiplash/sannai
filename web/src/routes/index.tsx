import { useQuery } from '@tanstack/react-query'
import { createFileRoute, Link } from '@tanstack/react-router'
import { sessionsQueryOptions } from '~/utils/queries'

export const Route = createFileRoute('/')({
  loader: ({ context }) =>
    context.queryClient.ensureQueryData(sessionsQueryOptions(5)).catch(() => []),
  component: Home,
})

function Home() {
  const { data: sessions = [], isError } = useQuery(sessionsQueryOptions(5))
  const active = sessions.filter((s) => !s.ended_at)

  return (
    <div className="p-6 max-w-4xl mx-auto space-y-8">
      <div>
        <h1 className="text-2xl font-bold mb-1">Sannai</h1>
        <p className="text-gray-600 dark:text-gray-400">
          AI code provenance — capture sessions, link to PRs.
        </p>
      </div>

      <div className="grid grid-cols-3 gap-4">
        <StatCard label="Total Sessions" value={sessions.length} />
        <StatCard label="Active" value={active.length} />
        <StatCard
          label="Events"
          value={sessions.reduce((sum, s) => sum + s.event_count, 0)}
        />
      </div>

      {sessions.length > 0 && (
        <div>
          <div className="flex items-center justify-between mb-3">
            <h2 className="text-lg font-semibold">Recent Sessions</h2>
            <Link
              to="/sessions"
              className="text-sm text-blue-600 dark:text-blue-400 hover:underline"
            >
              View all
            </Link>
          </div>
          <div className="space-y-2">
            {sessions.map((session) => (
              <Link
                key={session.id}
                to="/sessions/$sessionId"
                params={{ sessionId: session.id }}
                className="block p-3 rounded-lg border border-gray-200 dark:border-gray-800 hover:border-gray-300 dark:hover:border-gray-700 transition-colors"
              >
                <div className="flex items-center justify-between">
                  <span className="font-mono text-sm truncate max-w-xs">
                    {session.project_path || session.id}
                  </span>
                  <span className="text-xs text-gray-500">
                    {session.event_count} events
                  </span>
                </div>
                <div className="text-xs text-gray-500 mt-1">
                  {new Date(session.started_at).toLocaleString()}
                  {!session.ended_at && (
                    <span className="ml-2 text-emerald-600 dark:text-emerald-400 font-medium">
                      active
                    </span>
                  )}
                </div>
              </Link>
            ))}
          </div>
        </div>
      )}
    </div>
  )
}

function StatCard({ label, value }: { label: string; value: number }) {
  return (
    <div className="p-4 rounded-lg border border-gray-200 dark:border-gray-800">
      <div className="text-2xl font-bold">{value}</div>
      <div className="text-sm text-gray-500 dark:text-gray-400">{label}</div>
    </div>
  )
}
