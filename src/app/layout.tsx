import type { Metadata } from 'next';
import { DesktopWindowFrame } from '@/components/shell/DesktopWindowFrame';
import './globals.css';

export const metadata: Metadata = {
  title: 'Fusion Launcher',
  description: 'Your games. All in one place.'
};

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en">
      <body>
        <DesktopWindowFrame>{children}</DesktopWindowFrame>
      </body>
    </html>
  );
}
