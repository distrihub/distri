import { useEffect } from 'react';
import { BrowserRouter as Router, Routes, Route, Navigate, Outlet, useLocation } from 'react-router-dom';
import { DistriProvider, ThemeProvider } from '@distri/react';
import { TokenProvider, useInitialization } from '@/components/TokenProvider';
import { ThreadProvider } from '@/components/ThreadContext';
import { SessionProvider, useSession } from '@/components/SessionProvider';

// Import route components
import AuthPage from '@/routes/auth/AuthPage';
import AuthCallback from '@/routes/auth/AuthCallback';
import AuthSuccess from '@/routes/auth/AuthSuccess';
import LoginPage from '@/routes/login/LoginPage';
import HomeLayout from '@/layouts/HomeLayout';
import HomePage from '@/routes/home/HomePage';
import PaymentSuccess from '@/routes/payment/PaymentSuccess';
import AccountPage from '@/routes/home/menu/AccountPage';
import PricingPage from '@/routes/home/menu/PricingPage';
import HelpPage from '@/routes/home/menu/HelpPage';
import { BACKEND_URL } from './constants';
import { AccountProvider } from './components/AccountProvider';
import FilesPage from './routes/home/FilesPage';
import AgentsPage from './routes/home/AgentsPage';
import ThreadsPage from './routes/home/ThreadsPage';
import SettingsPage from './routes/home/SettingsPage';
import NewAgentPage from './routes/home/NewAgentPage';
import AgentDetailsPage from './routes/home/AgentDetailsPage';
import SessionsPage from './routes/home/SessionsPage';
import { Toaster } from './components/ui/sonner';

function App() {
  // Initialize theme to dark by default
  useEffect(() => {
    const currentTheme = localStorage.getItem('distri-theme');
    if (!currentTheme || currentTheme === 'system') {
      localStorage.setItem('distri-theme', 'dark');
    }
  }, []);

  const basePath = import.meta.env.BASE_URL || '/ui/';

  return (

    <ThemeProvider defaultTheme="dark" storageKey="distri-theme">
      <ThreadProvider>
        <Router basename={basePath}>
          <SessionProvider>

            <Routes>
              {/* Root redirect */}
              <Route path="/" element={<Navigate to="home" replace />} />

              {/* Auth routes */}
              <Route path="auth" element={<AuthPage />} />
              <Route path="auth/callback" element={<AuthCallback />} />
              <Route path="auth/success" element={<AuthSuccess />} />
              <Route path="login" element={<LoginPage />} />

              {/* Protected routes with layout */}
              <Route path="home" element={<LayoutWithProviders />}>
                <Route element={<HomeLayout />}>
                  <Route index element={<HomePage />} />
                  <Route path="agents" element={<AgentsPage />} />
                  <Route path="new" element={<NewAgentPage />} />
                  <Route path="agents/:agentId" element={<AgentDetailsPage />} />
                  <Route path="threads" element={<ThreadsPage />} />
                  <Route path="sessions" element={<SessionsPage />} />
                  <Route path="settings" element={<SettingsPage />} />
                  <Route path="menu/account" element={<AccountPage />} />
                  <Route path="menu/account/pricing" element={<PricingPage />} />
                  <Route path="menu/help" element={<HelpPage />} />
                </Route>
                <Route path="workspace" element={<FilesPage />} />
              </Route>

              {/* Payment routes */}
              <Route path="payment/success" element={<PaymentSuccess />} />

              {/* Catch all */}
              <Route path="*" element={<Navigate to="home" replace />} />
            </Routes>

            <Toaster position="top-right" richColors closeButton />
          </SessionProvider>
        </Router>
      </ThreadProvider>
    </ThemeProvider>
  );
}

const LayoutWithProviders = () => (
  <TokenProvider>
    <AccountProvider>
      <ProtectedLayout />
    </AccountProvider>
  </TokenProvider>
)

const ProtectedLayout = () => {
  const { token } = useInitialization()
  const { sessionId, loading: sessionLoading } = useSession()
  const location = useLocation()

  if (sessionLoading) {
    return <LoadingScreen message="Checking your device session..." />
  }

  if (!sessionId) {
    const redirectPath = `${location.pathname}${location.search}`
    return <Navigate to="/login" state={{ from: redirectPath }} replace />
  }

  const authHeaders = token ? { Authorization: `Bearer ${token}` } : undefined

  return (
    <DistriProvider
      config={{
        baseUrl: `${BACKEND_URL}/api/v1/`,
        headers: authHeaders,
        interceptor: async (init?: RequestInit): Promise<RequestInit | undefined> => {
          if (!token) {
            return init
          }
          const initCopy = init || {}
          const newInit = {
            ...initCopy,
            headers: {
              ...initCopy.headers,
              Authorization: `Bearer ${token}`,
            },
          }
          return newInit
        },
      }}
    >
      <Outlet />
    </DistriProvider>
  )
}

const LoadingScreen = ({ message }: { message: string }) => (
  <div className="min-h-screen bg-gray-50 dark:bg-gray-900 flex flex-col items-center justify-center">
    <div className="flex flex-col items-center gap-4">
      <div className="loading-spinner" />
      <p className="text-gray-600 dark:text-gray-400">{message}</p>
    </div>
  </div>
)



export default App;
