import { SidebarMenu, SidebarGroup, SidebarGroupContent, SidebarGroupLabel, SidebarGroupAction, SidebarMenuItem, SidebarMenuButton } from "@/components/ui/sidebar"
import { useThreads } from "@distri/react"
import { useNavigate, useSearchParams } from "react-router-dom"

import { Loader2, MessageCircle, RefreshCw, Workflow } from "lucide-react"

const Threads = () => {
  const { threads, loading: threadsLoading, refetch: refetchThreads } = useThreads()
  const navigate = useNavigate()

  const [searchParams] = useSearchParams()
  const threadId = searchParams.get('threadId')

  const handleRefresh = () => {
    refetchThreads()
  }

  // Get the active thread ID for the current spreadsheet

  const handleThreadSelect = (threadId: string, agentId: string, threadType?: string) => {
    // Don't allow navigation for workflow threads as they are read-only execution logs
    if (threadType === 'workflow') {
      return
    }

    if (agentId) {
      // Navigate to the chat page with the selected thread
      navigate(`/home/agents/${agentId}?threadId=${threadId}`, { replace: true })
    }
  }

  return (<SidebarGroup>
    <SidebarGroupLabel>Conversations</SidebarGroupLabel>
    <SidebarGroupContent>
      <SidebarMenu>
        {threadsLoading}
        {threadsLoading ? (
          <SidebarMenuItem>
            <SidebarMenuButton>
              <Loader2 className="h-4 w-4 animate-spin" />
              Loading threads...
            </SidebarMenuButton>
          </SidebarMenuItem>

        ) : threads.length === 0 ? (
          <SidebarMenuItem>
            <SidebarMenuButton>
              <MessageCircle className="h-4 w-4" />
              No conversations.
            </SidebarMenuButton>
          </SidebarMenuItem>
        ) :
          (
            threads.map((thread: any) => (
              <ThreadItem
                key={thread.id}
                thread={thread}
                isActive={thread.id === threadId}
                onClick={() => handleThreadSelect(thread.id, thread.agent_id, thread.type)}
                onDelete={() => { }} // Thread deletion handled by useThreads
              />
            ))
          )
        }
      </SidebarMenu>
    </SidebarGroupContent>

    {/* Refresh action for conversations */}
    {!threadsLoading && <SidebarGroupAction
      onClick={handleRefresh}
      disabled={threadsLoading}
      title="Refresh conversations"
    >
      <RefreshCw />
      <span className="sr-only">Refresh conversations</span>
    </SidebarGroupAction>}
  </SidebarGroup >)

}

export default Threads

interface ThreadItemProps {
  thread: any;
  isActive: boolean;
  onClick: () => void;
  onDelete: () => void;
}

const ThreadItem: React.FC<ThreadItemProps> = ({
  thread,
  isActive,
  onClick,
}) => {
  const isWorkflowThread = thread.type === 'workflow'

  return (
    <SidebarMenuItem className="mb-3">
      <SidebarMenuButton
        asChild
        isActive={isActive}
        disabled={isWorkflowThread}
        className={isWorkflowThread ? 'opacity-60 cursor-not-allowed' : ''}
      >
        <div onClick={isWorkflowThread ? undefined : onClick}>
          <div className="flex items-center gap-2 flex-1">
            {isWorkflowThread && <Workflow className="h-4 w-4 text-blue-500 flex-shrink-0" />}
            <div className="flex-1 min-w-0">
              <p className="text-sm font-medium truncate leading-tight">
                {thread.title || 'New Chat'}
                {isWorkflowThread && <span className="text-xs text-muted-foreground ml-1">(Execution Log)</span>}
              </p>
              <p className="text-xs text-muted-foreground truncate leading-tight mt-0.5">
                {thread.last_message || 'No messages yet'}
              </p>
            </div>
          </div>
        </div>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
};