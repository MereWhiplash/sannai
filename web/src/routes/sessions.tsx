import { useQuery } from '@tanstack/react-query'
import { createFileRoute, Outlet, useMatch } from '@tanstack/react-router'
import { sessionsQueryOptions } from '~/utils/queries'
import { SessionsTable } from '~/components/SessionsTable'

export const Route = createFileRoute('/sessions')({
  loader: ({ context }) =>
    context.queryClient.ensureQueryData(sessionsQueryOptions()).catch(() => []),
  component: Sessions,
})

function Sessions() {
  const { data: sessions = [] } = useQuery(sessionsQueryOptions())
  const childMatch = useMatch({ from: '/sessions/$sessionId', shouldThrow: false })

  if (childMatch) {
    return <Outlet />
  }

  return (
    <div className="p-6">
      <h1 className="text-2xl font-bold mb-4">Sessions</h1>
      {sessions.length === 0 ? (
        <p className="text-gray-600 dark:text-gray-400">
          No sessions captured yet. Start the agent with{' '}
          <code className="bg-gray-100 dark:bg-gray-800 px-1.5 py-0.5 rounded text-sm">
            make manual-test-live
          </code>
        </p>
      ) : (
        <SessionsTable sessions={sessions} />
      )}
    </div>
  )
}
