import { useQuery } from '@tanstack/react-query'
import { useStore } from '@tanstack/react-store'
import { useEffect } from 'react'
import { healthQueryOptions } from '~/utils/queries'
import { appStore, setAgentConnected } from '~/utils/store'

export function AgentStatus() {
  const { data, isError } = useQuery({
    ...healthQueryOptions(),
    refetchInterval: 15_000,
    retry: false,
  })

  const connected = useStore(appStore, (s) => s.agentConnected)

  useEffect(() => {
    setAgentConnected(!isError && !!data)
  }, [isError, data])

  return (
    <div className="flex items-center gap-2 text-xs">
      <span
        className={`inline-block w-2 h-2 rounded-full ${connected ? 'bg-emerald-500' : 'bg-red-500'}`}
      />
      <span className="text-gray-500 dark:text-gray-400">
        {connected ? `Agent v${data?.version}` : 'Agent offline'}
      </span>
    </div>
  )
}
