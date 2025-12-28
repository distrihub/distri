import { useEffect, useState, type CSSProperties } from 'react'
import { Outlet, useLocation, useNavigate, Link } from 'react-router-dom'
import { useTheme } from '@distri/react'
import Logo from "@/assets/logo.svg";
import LogoSmall from "@/assets/logo_small.svg";
import {
  Sidebar,
  useSidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarHeader,
  SidebarInset,
  SidebarMenu,
  SidebarMenuAction,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
  SidebarSeparator,
} from '@/components/ui/sidebar'
import { ChevronUp, LogOut, Settings, Users, Home, MessageSquare, User2, History } from 'lucide-react'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { Avatar, AvatarFallback, AvatarImage } from '@/components/ui/avatar'
import { useAccount } from '@/components/AccountProvider'

const navItems = [
  { id: 'home', label: 'Home', href: '/home', icon: Home },
  { id: 'agents', label: 'Agents', href: '/home/agents', icon: Users },
  { id: 'threads', label: 'Threads', href: '/home/threads', icon: MessageSquare },
  { id: 'sessions', label: 'Sessions', href: '/home/sessions', icon: History },
  { id: 'settings', label: 'Settings', href: '/home/settings', icon: Settings },
]

function LogoContainer() {
  const { open } = useSidebar();
  if (open) {
    return <img src={Logo} alt="Distri" className="h-6" />;
  } else {
    return <img src={LogoSmall} alt="Distri" className="h-6" />;
  }
}

export default function HomeLayout() {
  const [defaultOpen, setDefaultOpen] = useState(true)
  const sidebarStyles: CSSProperties = {
    '--sidebar-width': '20rem',
    '--sidebar-width-mobile': '18rem',
  }

  useEffect(() => {
    const savedState = localStorage.getItem('sidebar:state')
    if (savedState !== null) {
      setDefaultOpen(savedState === 'true')
    }
  }, [])

  return (
    <div className="flex h-screen min-h-0">
      <SidebarProvider defaultOpen={defaultOpen} style={sidebarStyles}>
        <HomeSidebar />
        <SidebarInset className="flex-1 min-h-0">
          <main className="flex h-full min-h-0 flex-1 overflow-hidden bg-background">
            <Outlet />
          </main>
        </SidebarInset>
      </SidebarProvider>
    </div>
  )
}


const HomeSidebar = () => {
  const { theme, setTheme } = useTheme()
  const { accountInfo } = useAccount()
  const navigate = useNavigate()
  const location = useLocation()

  const isActiveRoute = (path: string) =>
    location.pathname === path


  return (
    <Sidebar collapsible="icon" variant="sidebar">
      <SidebarHeader>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton onClick={() => navigate('/home')}>
              <LogoContainer />
            </SidebarMenuButton>
            <SidebarMenuAction
              onClick={() => setTheme(theme === 'light' ? 'dark' : 'light')}
              title="Toggle theme"
              className="absolute right-0 top-0"
            >
              <svg className="h-4 w-4 scale-100 rotate-0 transition-all dark:scale-0 dark:-rotate-90" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <circle cx="12" cy="12" r="5" />
                <path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />
              </svg>
              <svg className="absolute h-4 w-4 scale-0 rotate-90 transition-all dark:scale-100 dark:rotate-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
              </svg>
            </SidebarMenuAction>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarHeader>

      <SidebarSeparator />

      <SidebarContent>
        <SidebarGroup>
          <SidebarGroupContent>
            <SidebarMenu>
              {navItems.map((item) => (
                <SidebarMenuItem key={item.id} className="mb-1">
                  <SidebarMenuButton isActive={isActiveRoute(item.href)} onClick={() => navigate(item.href)}>
                    <item.icon className="h-4 w-4" />
                    {item.label}
                  </SidebarMenuButton>
                </SidebarMenuItem>
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>

      <SidebarFooter>
        <AccountMenuButton accountInfo={accountInfo} />
      </SidebarFooter>
    </Sidebar>
  )
}

// Threads list removed from sidebar per new design.

interface AccountMenuButtonProps {
  accountInfo: ReturnType<typeof useAccount>['accountInfo']
}

const AccountMenuButton = ({ accountInfo }: AccountMenuButtonProps) => (
  <SidebarMenu>
    <SidebarMenuItem>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <SidebarMenuButton size="lg" className="data-[state=open]:bg-sidebar-accent data-[state=open]:text-sidebar-accent-foreground">
            <Avatar className="h-8 w-8 rounded-lg">
              <AvatarImage src={accountInfo?.picture || undefined} alt={accountInfo?.email || 'User'} />
              <AvatarFallback className="rounded-lg">
                {accountInfo?.email?.charAt(0)?.toUpperCase() || <User2 className="h-4 w-4" />}
              </AvatarFallback>
            </Avatar>
            <div className="grid flex-1 text-left text-sm leading-tight">
              <span className="truncate text-xs">{accountInfo?.email}</span>
            </div>
            <ChevronUp className="ml-auto size-4" />
          </SidebarMenuButton>
        </DropdownMenuTrigger>
        <DropdownMenuContent className="w-[--radix-dropdown-menu-trigger-width] min-w-56 rounded-lg" side="bottom" align="end" sideOffset={4}>
          <DropdownMenuItem asChild>
            <Link to="/auth">
              <LogOut className="mr-2 h-4 w-4" />
              Log out
            </Link>
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </SidebarMenuItem>
  </SidebarMenu>
)
