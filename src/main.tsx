import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '@fontsource-variable/inter';
import '@fontsource-variable/orbitron';
import './styles/index.css';
import { App } from './app/App';
import { ErrorBoundary } from './components/ui/ErrorBoundary';
import { initializeAppearance } from './stores/themeStore';
import { initializeI18n } from './i18n';

initializeI18n();
const cleanup = initializeAppearance();
if (import.meta.hot) import.meta.hot.dispose(cleanup);

createRoot(document.getElementById('root')!).render(<StrictMode><ErrorBoundary><App /></ErrorBoundary></StrictMode>);
