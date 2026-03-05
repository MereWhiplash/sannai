/// <reference types="vite/client" />
import type { QueryClient } from '@tanstack/react-query'
import { QueryClientProvider } from '@tanstack/react-query'
import { ReactQueryDevtools } from '@tanstack/react-query-devtools'
import {
  HeadContent,
  Link,
  Outlet,
  Scripts,
  createRootRouteWithContext,
} from '@tanstack/react-router'
import { TanStackRouterDevtools } from '@tanstack/react-router-devtools'
import { DefaultCatchBoundary } from '~/components/DefaultCatchBoundary'
import { NotFound } from '~/components/NotFound'
import { AgentStatus } from '~/components/AgentStatus'
import appCss from '~/styles/app.css?url'
import { seo } from '~/utils/seo'

interface RouterContext {
  queryClient: QueryClient
}

export const Route = createRootRouteWithContext<RouterContext>()({
  head: () => ({
    meta: [
      { charSet: 'utf-8' },
      {
        name: 'viewport',
        content: 'width=device-width, initial-scale=1',
      },
      ...seo({
        title: 'Sannai | AI Code Provenance',
        description:
          'Capture AI coding sessions and link them to pull requests.',
      }),
    ],
    links: [{ rel: 'stylesheet', href: appCss }],
  }),
  errorComponent: DefaultCatchBoundary,
  notFoundComponent: () => <NotFound />,
  shellComponent: RootDocument,
  component: RootComponent,
})

function RootComponent() {
  const { queryClient } = Route.useRouteContext()
  return (
    <QueryClientProvider client={queryClient}>
      <div className="min-h-screen flex flex-col">
        <nav className="border-b border-gray-200 dark:border-gray-800 px-4 py-3 flex items-center gap-6">
          <Link
            to="/"
            className="font-bold text-lg"
            activeOptions={{ exact: true }}
          >
            Sannai
          </Link>
          <div className="flex gap-4 text-sm">
            <Link
              to="/sessions"
              activeProps={{ className: 'font-semibold text-gray-900 dark:text-gray-100' }}
              className="text-gray-600 dark:text-gray-400 hover:text-gray-900 dark:hover:text-gray-100"
            >
              Sessions
            </Link>
          </div>
          <div className="ml-auto">
            <AgentStatus />
          </div>
        </nav>
        <main className="flex-1">
          <Outlet />
        </main>
      </div>
      <ReactQueryDevtools buttonPosition="bottom-left" />
      <TanStackRouterDevtools position="bottom-right" />
    </QueryClientProvider>
  )
}

function RootDocument({ children }: { children: React.ReactNode }) {
  return (
    <html>
      <head>
        <HeadContent />
      </head>
      <body>
        {children}
        <Scripts />
      </body>
    </html>
  )
}
