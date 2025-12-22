import React, { useState, useCallback } from "react";
import { useThreads, useTheme } from "@distri/react";
import {
  MoreHorizontal,
  Trash2,
  Edit3,
  Users,
  Edit2,
  RefreshCw,
  Github,
  Loader2,
} from "lucide-react";
import Logo from "@/assets/logo_small.svg";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarGroupAction,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuAction,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarSeparator,
} from "./ui/sidebar";
import { Input } from "@/components/ui/input";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useSidebar } from "@/components/ui/sidebar";

interface ThreadItemProps {
  thread: any;
  isActive: boolean;
  onClick: () => void;
  onDelete: () => void;
  onRename: (newTitle: string) => void;
}

const ThreadItem: React.FC<ThreadItemProps> = ({
  thread,
  isActive,
  onClick,
  onDelete,
  onRename,
}) => {
  const [isEditing, setIsEditing] = useState(false);
  const [editTitle, setEditTitle] = useState(thread.title || "New Chat");
  const [showMenu, setShowMenu] = useState(false);

  const handleRename = useCallback(() => {
    if (editTitle.trim() && editTitle !== thread.title) {
      onRename(editTitle.trim());
    }
    setIsEditing(false);
  }, [editTitle, thread.title, onRename]);

  const handleKeyPress = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        handleRename();
      } else if (e.key === "Escape") {
        setEditTitle(thread.title || "New Chat");
        setIsEditing(false);
      }
    },
    [handleRename, thread.title],
  );

  return (
    <SidebarMenuItem className="mb-3">
      <SidebarMenuButton asChild isActive={isActive}>
        <div onClick={onClick}>
          {isEditing ? (
            <Input
              value={editTitle}
              onChange={(e) => setEditTitle(e.target.value)}
              onBlur={handleRename}
              onKeyPress={handleKeyPress}
              className="flex-1 text-sm bg-transparent border-none outline-none"
              autoFocus
              onClick={(e) => e.stopPropagation()}
            />
          ) : (
            <div className="flex-1">
              <p className="text-sm font-medium truncate leading-tight">
                {thread.title || "New Chat"}
              </p>
              <p className="text-xs text-muted-foreground truncate leading-tight mt-0.5">
                {thread.last_message || "No messages yet"}
              </p>
            </div>
          )}
        </div>
      </SidebarMenuButton>

      {!isEditing && (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <SidebarMenuAction
              onClick={(e) => {
                e.stopPropagation();
                setShowMenu(!showMenu);
              }}
            >
              <MoreHorizontal />
            </SidebarMenuAction>
          </DropdownMenuTrigger>
          <DropdownMenuContent className="w-[--radix-popper-anchor-width]">
            <DropdownMenuItem
              onClick={(e) => {
                e.stopPropagation();
                setIsEditing(true);
                setShowMenu(false);
              }}
            >
              <Edit3 className="h-3 w-3" />
              <span>Rename</span>
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={(e) => {
                e.stopPropagation();
                onDelete();
                setShowMenu(false);
              }}
            >
              <Trash2 className="h-3 w-3" />
              <span>Delete</span>
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      )}
    </SidebarMenuItem>
  );
};

interface AppSidebarProps {
  selectedThreadId: string;
  currentPage: "chat" | "agents";
  onNewChat: () => void;
  onThreadSelect: (threadId: string) => void;
  onThreadDelete: (threadId: string) => void;
  onThreadRename: (threadId: string, newTitle: string) => void;
  onLogoClick?: () => void;
  onPageChange: (page: "chat" | "agents") => void;
}

export function AppSidebar({
  selectedThreadId,
  currentPage,
  onNewChat,
  onThreadSelect,
  onThreadDelete,
  onThreadRename,
  onLogoClick,
  onPageChange,
}: AppSidebarProps) {
  const { threads, loading: threadsLoading, refetch } = useThreads();
  const { theme, setTheme } = useTheme();

  const { open } = useSidebar();
  const handleRefresh = useCallback(() => {
    refetch();
  }, [refetch]);

  return (
    <Sidebar collapsible="icon" variant="floating">
      <SidebarHeader>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton onClick={onLogoClick} className="gap-2">
              <img src={Logo} alt="Distri" className="h-6 w-6" />
              <span className="font-semibold text-base tracking-tight">
                Distri
              </span>
            </SidebarMenuButton>
            <SidebarMenuAction
              onClick={() => setTheme(theme === "light" ? "dark" : "light")}
              title="Toggle theme"
              className="absolute right-0 top-0"
            >
              <svg
                className="h-4 w-4 scale-100 rotate-0 transition-all dark:scale-0 dark:-rotate-90"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
              >
                <circle cx="12" cy="12" r="5" />
                <path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />
              </svg>
              <svg
                className="absolute h-4 w-4 scale-0 rotate-90 transition-all dark:scale-100 dark:rotate-0"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
              >
                <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
              </svg>
            </SidebarMenuAction>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarHeader>

      <SidebarSeparator />

      <SidebarContent>
        <SidebarGroup>
          <SidebarGroupLabel>Actions</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              <SidebarMenuItem className="mb-1">
                <SidebarMenuButton
                  isActive={currentPage === "chat"}
                  onClick={() => {
                    onPageChange("chat");
                    onNewChat();
                  }}
                >
                  <Edit2 className="h-4 w-4" />
                  New Chat
                </SidebarMenuButton>
              </SidebarMenuItem>
              <SidebarMenuItem className="mb-1">
                <SidebarMenuButton
                  isActive={currentPage === "agents"}
                  onClick={() => onPageChange("agents")}
                >
                  <Users className="h-4 w-4" />
                  Agents
                </SidebarMenuButton>
              </SidebarMenuItem>
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>

        {open && (
          <SidebarGroup>
            <SidebarGroupLabel>Conversations</SidebarGroupLabel>
            <SidebarGroupContent>
              <SidebarMenu>
                {threadsLoading ? (
                  <SidebarMenuItem>
                    <Loader2 className="h-4 w-4 animate-spin" />
                    <span>Loading threads...</span>
                  </SidebarMenuItem>
                ) : threads.length === 0 ? (
                  <SidebarMenuItem>No conversations yet</SidebarMenuItem>
                ) : (
                  threads.map((thread: any) => (
                    <ThreadItem
                      key={thread.id}
                      thread={thread}
                      isActive={thread.id === selectedThreadId}
                      onClick={() => onThreadSelect(thread.id)}
                      onDelete={() => onThreadDelete(thread.id)}
                      onRename={(newTitle) =>
                        onThreadRename(thread.id, newTitle)
                      }
                    />
                  ))
                )}
              </SidebarMenu>
            </SidebarGroupContent>

            {/* Refresh action for conversations */}
            <SidebarGroupAction
              onClick={handleRefresh}
              disabled={threadsLoading}
              title="Refresh conversations"
            >
              <RefreshCw
                className={`${threadsLoading ? "animate-spin" : ""}`}
              />
              <span className="sr-only">Refresh conversations</span>
            </SidebarGroupAction>
          </SidebarGroup>
        )}
      </SidebarContent>

      <SidebarFooter>
        <SidebarMenu>
          <SidebarMenuItem>
            {/* Distri logo and GitHub link */}

            <SidebarMenuButton
              onClick={() =>
                window.open("https://github.com/your-repo/distri", "_blank")
              }
              title="GitHub"
            >
              <Github />
              Distri
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>
    </Sidebar>
  );
}
