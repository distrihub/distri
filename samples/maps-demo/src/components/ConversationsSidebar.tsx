import { MapPin, RefreshCw, Loader2, Plus, Trash2, ColumnsIcon } from 'lucide-react';
import {
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuAction,
  SidebarGroupAction,
  SidebarSeparator,
  useSidebar,
} from '@distri/components';

interface ConversationsSidebarProps {
  threads: any[];
  selectedThreadId: string;
  loading: boolean;
  onThreadSelect: (threadId: string) => void;
  onThreadDelete: (threadId: string) => void;
  onRefresh: () => void;
  onNewChat: () => void;
}

export function ConversationsSidebar({
  threads,
  selectedThreadId,
  loading,
  onThreadSelect,
  onThreadDelete,
  onRefresh,
  onNewChat,
}: ConversationsSidebarProps) {
  const { open, setOpen } = useSidebar();

  return (
    <Sidebar collapsible="icon" variant="floating" >
      <SidebarHeader>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton>
              <MapPin />
              Maps Navigator
            </SidebarMenuButton>
            <SidebarMenuAction onClick={() => setOpen(false)}>

              <ColumnsIcon />

            </SidebarMenuAction>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarHeader>
      <SidebarSeparator />
      <SidebarContent>
        {!open && (
          <SidebarGroup>
            <SidebarGroupContent>
              <SidebarMenu>
                <SidebarMenuItem>
                  <SidebarMenuButton onClick={() => setOpen(true)}>
                    <ColumnsIcon />
                  </SidebarMenuButton>
                </SidebarMenuItem>
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        )}
        <SidebarGroup>
          <SidebarGroupLabel>Actions</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>

              <SidebarMenuItem className="mb-1">
                <SidebarMenuButton onClick={onNewChat}>
                  <Plus />
                  New Chat
                </SidebarMenuButton>
              </SidebarMenuItem>
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>

        {open && <SidebarGroup>
          <SidebarGroupLabel>Conversations</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {loading ? (
                <SidebarMenuItem>
                  <Loader2 className="h-4 w-4 animate-spin" />
                  <span>Loading conversations...</span>
                </SidebarMenuItem>
              ) : threads.length === 0 ? (
                <SidebarMenuItem>
                  <SidebarMenuButton disabled>
                    No conversations yet
                  </SidebarMenuButton>
                </SidebarMenuItem>
              ) : (
                threads.map((thread: any) => (
                  <SidebarMenuItem key={thread.id}>
                    <SidebarMenuButton
                      isActive={thread.id === selectedThreadId}
                      onClick={() => onThreadSelect(thread.id)}
                    >
                      <span>{thread.title || 'New Chat'}</span>
                    </SidebarMenuButton>
                    <SidebarMenuAction onClick={() => onThreadDelete(thread.id)}>
                      <Trash2 />
                    </SidebarMenuAction>
                  </SidebarMenuItem>
                ))
              )}
            </SidebarMenu>
          </SidebarGroupContent>
          <SidebarGroupAction
            onClick={onRefresh}
            disabled={loading}
            title="Refresh conversations"
          >
            <RefreshCw className={`${loading ? 'animate-spin' : ''}`} />
            <span className="sr-only">Refresh conversations</span>
          </SidebarGroupAction>
        </SidebarGroup>}
      </SidebarContent>
    </Sidebar>
  );
} 