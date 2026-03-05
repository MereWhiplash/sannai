import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/sessions')({
  component: Sessions,
})

function Sessions() {
  return (
    <div className="p-6">
      <h1 className="text-2xl font-bold mb-4">Sessions</h1>
      <p className="text-gray-600 dark:text-gray-400">
        No sessions captured yet.
      </p>
    </div>
  )
}
