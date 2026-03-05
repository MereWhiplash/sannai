import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/')({
  component: Home,
})

function Home() {
  return (
    <div className="p-6">
      <h1 className="text-2xl font-bold mb-2">Sannai</h1>
      <p className="text-gray-600 dark:text-gray-400">
        AI code provenance — capture sessions, link to PRs.
      </p>
    </div>
  )
}
