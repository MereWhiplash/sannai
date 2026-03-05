import type { SessionEvent } from '~/utils/api'

const typeStyles: Record<string, { label: string; color: string }> = {
  user_prompt: { label: 'Prompt', color: 'bg-blue-500' },
  assistant_response: { label: 'Response', color: 'bg-purple-500' },
  tool_use: { label: 'Tool', color: 'bg-amber-500' },
  tool_result: { label: 'Result', color: 'bg-gray-400' },
}

export function EventTimeline({ events }: { events: SessionEvent[] }) {
  if (events.length === 0) {
    return (
      <p className="text-gray-500 dark:text-gray-400">
        No events recorded for this session.
      </p>
    )
  }

  return (
    <div className="space-y-0">
      {events.map((event, i) => {
        const style = typeStyles[event.event_type] ?? {
          label: event.event_type,
          color: 'bg-gray-400',
        }
        return (
          <div key={event.id ?? i} className="flex gap-3 group">
            <div className="flex flex-col items-center">
              <div className={`w-2.5 h-2.5 rounded-full mt-1.5 ${style.color}`} />
              {i < events.length - 1 && (
                <div className="w-px flex-1 bg-gray-200 dark:bg-gray-800" />
              )}
            </div>
            <div className="pb-4 min-w-0 flex-1">
              <div className="flex items-center gap-2 mb-0.5">
                <span className="text-xs font-medium">{style.label}</span>
                <span className="text-xs text-gray-400">
                  {new Date(event.timestamp).toLocaleTimeString()}
                </span>
                {event.metadata && 'model' in event.metadata && (
                  <span className="text-xs text-gray-400">
                    {String(event.metadata.model)}
                  </span>
                )}
              </div>
              {event.content && (
                <pre className="text-sm text-gray-700 dark:text-gray-300 whitespace-pre-wrap break-words max-h-40 overflow-y-auto font-mono bg-gray-50 dark:bg-gray-900 rounded p-2">
                  {event.content}
                </pre>
              )}
            </div>
          </div>
        )
      })}
    </div>
  )
}
