import { useState } from 'react'
import type { SessionEvent } from '~/utils/api'

interface TimelineEntry {
  type: 'prompt' | 'response' | 'tools'
  timestamp: string
  events: SessionEvent[]
}

function groupEvents(events: SessionEvent[]): TimelineEntry[] {
  const entries: TimelineEntry[] = []

  let i = 0
  while (i < events.length) {
    const event = events[i]

    if (event.event_type === 'user_prompt') {
      entries.push({ type: 'prompt', timestamp: event.timestamp, events: [event] })
      i++
    } else if (event.event_type === 'assistant_response') {
      entries.push({ type: 'response', timestamp: event.timestamp, events: [event] })
      i++
    } else if (event.event_type === 'tool_use' || event.event_type === 'tool_result') {
      // Collect consecutive tool_use/tool_result pairs into one group
      const toolEvents: SessionEvent[] = []
      while (
        i < events.length &&
        (events[i].event_type === 'tool_use' || events[i].event_type === 'tool_result')
      ) {
        toolEvents.push(events[i])
        i++
      }
      entries.push({ type: 'tools', timestamp: toolEvents[0].timestamp, events: toolEvents })
    } else {
      i++
    }
  }

  return entries
}

function ToolGroup({ events }: { events: SessionEvent[] }) {
  const [expanded, setExpanded] = useState(false)
  const toolUses = events.filter((e) => e.event_type === 'tool_use')
  const toolNames = toolUses.map((e) => e.content || 'unknown')

  // Count tool calls by name
  const counts: Record<string, number> = {}
  for (const name of toolNames) {
    counts[name] = (counts[name] || 0) + 1
  }

  const summary = Object.entries(counts)
    .map(([name, count]) => (count > 1 ? `${name} x${count}` : name))
    .join(', ')

  const errorResults = events.filter(
    (e) => e.event_type === 'tool_result' && e.metadata && (e.metadata as Record<string, unknown>).is_error === true,
  )

  return (
    <div>
      <button
        onClick={() => setExpanded(!expanded)}
        className="flex items-center gap-2 text-sm w-full text-left hover:bg-gray-50 dark:hover:bg-gray-800/50 rounded px-1.5 py-0.5 -mx-1.5 transition-colors"
      >
        <span className="text-xs text-gray-400">{expanded ? '\u25BC' : '\u25B6'}</span>
        <span className="font-mono text-xs text-amber-700 dark:text-amber-400">
          {summary}
        </span>
        <span className="text-xs text-gray-400">
          {toolUses.length} call{toolUses.length !== 1 ? 's' : ''}
        </span>
        {errorResults.length > 0 && (
          <span className="text-xs text-red-500 font-medium">
            {errorResults.length} error{errorResults.length !== 1 ? 's' : ''}
          </span>
        )}
      </button>
      {expanded && (
        <div className="mt-1 ml-4 space-y-1">
          {toolUses.map((tool, i) => (
            <div key={tool.id ?? i} className="text-xs font-mono text-gray-500 dark:text-gray-400">
              {tool.content || 'unknown'}
              <span className="text-gray-400 ml-2">
                {new Date(tool.timestamp).toLocaleTimeString()}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

function cleanPromptContent(content: string): string {
  // Strip XML-like system tags that clutter the display
  return content
    .replace(/<command-name>.*?<\/command-name>\s*/gs, '')
    .replace(/<command-message>.*?<\/command-message>\s*/gs, '')
    .replace(/<command-args>.*?<\/command-args>\s*/gs, '')
    .replace(/<local-command-caveat>.*?<\/local-command-caveat>\s*/gs, '')
    .replace(/<system-reminder>.*?<\/system-reminder>\s*/gs, '')
    .trim()
}

const entryStyles = {
  prompt: { label: 'Prompt', dot: 'bg-blue-500', accent: 'border-blue-200 dark:border-blue-900' },
  response: { label: 'Response', dot: 'bg-purple-500', accent: 'border-purple-200 dark:border-purple-900' },
  tools: { label: 'Tools', dot: 'bg-amber-500', accent: 'border-amber-200 dark:border-amber-900' },
}

export function EventTimeline({ events }: { events: SessionEvent[] }) {
  if (events.length === 0) {
    return (
      <p className="text-gray-500 dark:text-gray-400">
        No events recorded for this session.
      </p>
    )
  }

  const entries = groupEvents(events)

  return (
    <div className="space-y-0">
      {entries.map((entry, i) => {
        const style = entryStyles[entry.type]
        return (
          <div key={i} className="flex gap-3">
            <div className="flex flex-col items-center">
              <div className={`w-2.5 h-2.5 rounded-full mt-1.5 flex-shrink-0 ${style.dot}`} />
              {i < entries.length - 1 && (
                <div className="w-px flex-1 bg-gray-200 dark:bg-gray-800" />
              )}
            </div>
            <div className="pb-4 min-w-0 flex-1">
              <div className="flex items-center gap-2 mb-0.5">
                <span className="text-xs font-medium">{style.label}</span>
                <span className="text-xs text-gray-400">
                  {new Date(entry.timestamp).toLocaleTimeString()}
                </span>
                {entry.type === 'response' && entry.events[0].metadata && (
                  <TokenBadge metadata={entry.events[0].metadata} />
                )}
              </div>
              <EntryContent entry={entry} />
            </div>
          </div>
        )
      })}
    </div>
  )
}

function TokenBadge({ metadata }: { metadata: Record<string, unknown> }) {
  const input = metadata.input_tokens as number | undefined
  const output = metadata.output_tokens as number | undefined
  if (!input && !output) return null

  return (
    <span className="text-xs text-gray-400 font-mono">
      {input ? `${(input / 1000).toFixed(1)}k in` : ''}
      {input && output ? ' / ' : ''}
      {output ? `${(output / 1000).toFixed(1)}k out` : ''}
    </span>
  )
}

function EntryContent({ entry }: { entry: TimelineEntry }) {
  const [expanded, setExpanded] = useState(false)

  if (entry.type === 'tools') {
    return <ToolGroup events={entry.events} />
  }

  const event = entry.events[0]
  const raw = event.content || ''
  const content = entry.type === 'prompt' ? cleanPromptContent(raw) : raw

  if (!content) return null

  const isLong = content.length > 300
  const displayed = isLong && !expanded ? content.slice(0, 300) + '...' : content

  return (
    <div>
      <pre className="text-sm text-gray-700 dark:text-gray-300 whitespace-pre-wrap break-words font-mono bg-gray-50 dark:bg-gray-900 rounded p-2 max-h-60 overflow-y-auto">
        {displayed}
      </pre>
      {isLong && (
        <button
          onClick={() => setExpanded(!expanded)}
          className="text-xs text-blue-500 hover:underline mt-1"
        >
          {expanded ? 'Show less' : 'Show more'}
        </button>
      )}
    </div>
  )
}
