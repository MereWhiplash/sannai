import {
  createColumnHelper,
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  getFilteredRowModel,
  useReactTable,
  type SortingState,
} from '@tanstack/react-table'
import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import type { Session } from '~/utils/api'

const col = createColumnHelper<Session>()

const columns = [
  col.accessor('project_path', {
    header: 'Project',
    cell: (info) => (
      <Link
        to="/sessions/$sessionId"
        params={{ sessionId: info.row.original.id }}
        className="text-blue-600 dark:text-blue-400 hover:underline font-mono text-sm truncate block max-w-xs"
      >
        {info.getValue() || info.row.original.id.slice(0, 8)}
      </Link>
    ),
  }),
  col.accessor('tool', {
    header: 'Tool',
    cell: (info) => (
      <span className="text-xs px-2 py-0.5 rounded bg-gray-100 dark:bg-gray-800">
        {info.getValue()}
      </span>
    ),
  }),
  col.accessor('started_at', {
    header: 'Started',
    cell: (info) => new Date(info.getValue()).toLocaleString(),
    sortingFn: 'datetime',
  }),
  col.accessor('ended_at', {
    header: 'Status',
    cell: (info) =>
      info.getValue() ? (
        <span className="text-gray-500 text-xs">
          Ended {new Date(info.getValue()!).toLocaleString()}
        </span>
      ) : (
        <span className="text-emerald-600 dark:text-emerald-400 text-xs font-medium">
          Active
        </span>
      ),
  }),
  col.accessor('event_count', {
    header: 'Events',
    cell: (info) => info.getValue(),
  }),
]

export function SessionsTable({ sessions }: { sessions: Session[] }) {
  const [sorting, setSorting] = useState<SortingState>([
    { id: 'started_at', desc: true },
  ])
  const [globalFilter, setGlobalFilter] = useState('')

  const table = useReactTable({
    data: sessions,
    columns,
    state: { sorting, globalFilter },
    onSortingChange: setSorting,
    onGlobalFilterChange: setGlobalFilter,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getFilteredRowModel: getFilteredRowModel(),
  })

  return (
    <div className="space-y-3">
      <input
        type="text"
        value={globalFilter}
        onChange={(e) => setGlobalFilter(e.target.value)}
        placeholder="Filter sessions..."
        className="px-3 py-1.5 border border-gray-300 dark:border-gray-700 rounded-md bg-white dark:bg-gray-900 text-sm w-64"
      />
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            {table.getHeaderGroups().map((headerGroup) => (
              <tr key={headerGroup.id} className="border-b border-gray-200 dark:border-gray-800">
                {headerGroup.headers.map((header) => (
                  <th
                    key={header.id}
                    className="text-left py-2 px-3 font-medium text-gray-600 dark:text-gray-400 cursor-pointer select-none"
                    onClick={header.column.getToggleSortingHandler()}
                  >
                    <span className="flex items-center gap-1">
                      {flexRender(header.column.columnDef.header, header.getContext())}
                      {{ asc: ' \u2191', desc: ' \u2193' }[
                        header.column.getIsSorted() as string
                      ] ?? ''}
                    </span>
                  </th>
                ))}
              </tr>
            ))}
          </thead>
          <tbody>
            {table.getRowModel().rows.map((row) => (
              <tr
                key={row.id}
                className="border-b border-gray-100 dark:border-gray-800/50 hover:bg-gray-50 dark:hover:bg-gray-900/50"
              >
                {row.getVisibleCells().map((cell) => (
                  <td key={cell.id} className="py-2 px-3">
                    {flexRender(cell.column.columnDef.cell, cell.getContext())}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="text-xs text-gray-500">
        {table.getFilteredRowModel().rows.length} session(s)
      </div>
    </div>
  )
}
