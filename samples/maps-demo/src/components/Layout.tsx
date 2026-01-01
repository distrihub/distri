import React from 'react';
import { SidebarProvider, SidebarInset } from '@distri/components';
import GoogleMapsManager, { GoogleMapsManagerRef } from './GoogleMapsManager';
import { ConversationsSidebar } from './ConversationsSidebar';

interface LayoutProps {
  children: React.ReactNode;
  onMapReady: (mapRef: GoogleMapsManagerRef) => void;
  threads: Array<{ id: string; title?: string; created_at?: string }>; // minimal shape used by ConversationsSidebar
  selectedThreadId: string;
  loading: boolean;
  onThreadSelect: (threadId: string) => void;
  onThreadDelete: (threadId: string) => Promise<void>;
  onRefresh: () => Promise<void> | void;
  onNewChat: () => void;
}

const Layout: React.FC<LayoutProps> = ({
  children,
  onMapReady,
  threads,
  selectedThreadId,
  loading,
  onThreadSelect,
  onThreadDelete,
  onRefresh,
  onNewChat,
}) => {
  return (
    <SidebarProvider
      defaultOpen={true}
      style={{
        "--sidebar-width": "20rem",
        "--sidebar-width-mobile": "18rem",
      } as any}
    >
      <ConversationsSidebar
        threads={threads}
        selectedThreadId={selectedThreadId}
        loading={loading}
        onThreadSelect={onThreadSelect}
        onThreadDelete={onThreadDelete}
        onRefresh={onRefresh}
        onNewChat={onNewChat}
      />
      <SidebarInset>
        <main className="flex-1 overflow-hidden">
          <div className="flex h-screen">
            {/* Google Maps Panel */}
            <div className="flex-1">
              <GoogleMapsManager
                defaultCenter={{ lat: 37.7749, lng: -122.4194 }}
                defaultZoom={13}
                onReady={onMapReady}
              />
            </div>

            {/* Chat Panel */}
            <div className="w-96">
              <div className="h-full">
                {children}
              </div>
            </div>
          </div>
        </main>
      </SidebarInset>
    </SidebarProvider>
  );
};

export default Layout;


