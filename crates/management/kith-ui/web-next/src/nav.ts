import { Activity, BarChart3, LayoutDashboard, Server, Settings, type LucideIcon } from 'lucide-react';

export type NavItem = { path: string; label: string; icon: LucideIcon; question: string };

/** The five places, each answering one question (DESIGN.md). */
export const NAV: readonly NavItem[] = [
  { path: '/', label: 'Overview', icon: LayoutDashboard, question: 'Is everything okay?' },
  { path: '/devices', label: 'Devices', icon: Server, question: 'What is on the network?' },
  { path: '/stats', label: 'Stats', icon: BarChart3, question: 'How is it performing?' },
  { path: '/activity', label: 'Activity', icon: Activity, question: 'What happened?' },
  { path: '/settings', label: 'Settings', icon: Settings, question: 'How is it set up?' },
];

export const SETTINGS: readonly { slug: string; label: string; description: string }[] = [
  { slug: 'network', label: 'Network', description: 'Invites, relays and this device' },
  { slug: 'exits', label: 'Exits', description: 'Who may send internet traffic through whom' },
  { slug: 'names', label: 'Names', description: 'DNS names on the network' },
  { slug: 'access', label: 'Access', description: 'Which devices may reach which' },
  { slug: 'people', label: 'People', description: 'Who can sign in to this Console' },
];

export const isActive = (item: NavItem, path: string) =>
  item.path === '/' ? path === '/' : path === item.path || path.startsWith(`${item.path}/`);
