import React from 'react';
import { SidebarProvider, SidebarInset } from '@distri/components';
import GoogleMapsManager, { GoogleMapsManagerRef } from './GoogleMapsManager';
import { ConversationsSidebar } from './ConversationsSidebar';

interface LayoutProps {
  children: React.ReactNode;
  onMapReady: (mapRef: GoogleMapsManagerRef) => void;
  onNewChat: () => void;
}

const Layout: React.FC<LayoutProps> = ({
  children,
  onMapReady,
  onNewChat,
}) => {
  return (
    <SidebarProvider
      defaultOpen={false}
      style={{
        "--sidebar-width": "3rem",
        "--sidebar-width-mobile": "3rem",
      } as any}
    >
      <ConversationsSidebar onNewChat={onNewChat} />
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


