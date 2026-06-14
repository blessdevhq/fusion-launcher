/** @type {import('tailwindcss').Config} */
export default {
  content: ['./src/app/**/*.{ts,tsx}', './src/components/**/*.{ts,tsx}', './src/lib/**/*.{ts,tsx}'],
  theme: {
    extend: {
      fontFamily: {
        sans: ['Inter', 'ui-sans-serif', 'system-ui', 'Segoe UI', 'sans-serif']
      },
      colors: {
        fusion: {
          bg: '#0A0A0B',
          surface: '#141416',
          raised: '#1E1E21',
          line: '#2A2A2E',
          accent: '#5CE68C',
          accentHover: '#4BD67C',
          accentActive: '#3FBF6E',
          accentOn: '#06210F',
          green: '#5CE68C',
          cyan: '#3BD6C6'
        }
      },
      boxShadow: {
        glow: '0 12px 30px rgba(92, 230, 140, 0.22)'
      }
    }
  },
  plugins: []
};
