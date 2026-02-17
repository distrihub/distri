import { ComponentType } from 'react';
import { ConfigurationPanel } from '@distri/react';
import { useDistriHomeNavigate } from '../DistriHomeProvider';
import { SecretsView } from './SecretsView';
import { SkillsView } from './SkillsView';
import { Settings as SettingsIcon, LockIcon, CodeIcon, LucideIcon } from 'lucide-react';

// Section type definition - exported for consumers to create custom sections
export interface SettingsSection {
  id: string;
  label: string;
  icon: LucideIcon;
  href: string;
  section: ComponentType<{ className?: string }>;
}

export interface SettingsViewProps {
  /**
   * Optional custom class name
   */
  className?: string;
  /**
   * Active section (tab)
   */
  activeSection?: string;
  /**
   * Custom sections to display. If not provided, uses defaultSections.
   * Cloud applications can pass their own sections (Account, API Keys, etc.)
   */
  sections?: SettingsSection[];
  /**
   * Callback when section changes
   */
  onSectionChange?: (section: string) => void;
}

// Wrapper for ConfigurationPanel to match section component signature
function ConfigurationSection({ className }: { className?: string }) {
  return (
    <div className={`space-y-6 ${className ?? ''}`}>
      <div className="rounded-2xl border border-border/70 bg-card p-6 shadow-sm">
        <ConfigurationPanel title="Agent settings" />
      </div>
    </div>
  );
}

// Default sections - core distri-home sections only (no cloud-specific Account/API Keys)
export const defaultSections: SettingsSection[] = [
  { id: 'configuration', label: 'Configuration', icon: SettingsIcon, href: 'settings', section: ConfigurationSection },
  { id: 'secrets', label: 'Secrets', icon: LockIcon, href: 'settings/secrets', section: SecretsView },
  { id: 'skills', label: 'Skills', icon: CodeIcon, href: 'settings/skills', section: SkillsView },
];

// Empty fallback section
function EmptySection() {
  return <div className="text-muted-foreground text-sm">Section not found.</div>;
}

export function SettingsView({
  className,
  activeSection,
  onSectionChange,
  sections,
}: SettingsViewProps) {
  const navigate = useDistriHomeNavigate();

  const setActiveSection = (section: string) => {
    onSectionChange?.(section);
  };

  // Use provided sections or default sections
  const tabs = sections ?? defaultSections;

  // Find active section component
  const activeTab = tabs.find(t => t.id === activeSection);
  const SectionComponent = activeTab?.section ?? EmptySection;

  return (
    <div className={`flex-1 overflow-y-auto ${className ?? ''}`}>
      <div className="mx-auto flex w-full max-w-5xl flex-col px-6 py-8 lg:px-10">
        <div className="mb-6">
          <p className="text-xs font-semibold uppercase tracking-[0.3em] text-primary">Settings</p>
          <p className="mt-2 text-sm text-muted-foreground">
            Manage configuration and settings.
          </p>
        </div>

        <div className="border-b border-border/60">
          <nav className="-mb-px flex flex-wrap gap-6 text-sm font-medium text-muted-foreground">
            {tabs.map(({ id, label: tabLabel, icon: Icon, href }) => (
              <button
                key={id}
                type="button"
                onClick={() => {
                  setActiveSection(id);
                  if (href) {
                    navigate(href);
                  }
                }}
                className={`flex items-center gap-2 border-b-2 px-1 py-3 transition ${activeSection === id
                  ? 'border-primary text-primary'
                  : 'border-transparent hover:border-border/80 hover:text-foreground'
                  }`}
              >
                <Icon className="h-4 w-4" />
                {tabLabel}
              </button>
            ))}
          </nav>
        </div>

        <div className="mt-6">
          <SectionComponent />
        </div>
      </div>
    </div>
  );
}
