/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        ink: {
          50: "#f4f7f5",
          100: "#e3ebe6",
          200: "#c5d6cb",
          300: "#9bb8a6",
          400: "#6f947f",
          500: "#4f7662",
          600: "#3c5d4c",
          700: "#314b3e",
          800: "#293d34",
          900: "#22332b",
          950: "#121c18",
        },
        ember: {
          400: "#e8a35c",
          500: "#d4843a",
          600: "#b86828",
        },
      },
      fontFamily: {
        display: ["\"Fraunces\"", "Georgia", "serif"],
        sans: ["\"IBM Plex Sans\"", "ui-sans-serif", "system-ui", "sans-serif"],
        mono: ["\"IBM Plex Mono\"", "ui-monospace", "monospace"],
      },
      backgroundImage: {
        mesh: "radial-gradient(ellipse at 20% 0%, rgba(212,132,58,0.18), transparent 50%), radial-gradient(ellipse at 80% 20%, rgba(79,118,98,0.35), transparent 45%), linear-gradient(160deg, #121c18 0%, #1a2a23 40%, #0f1613 100%)",
      },
    },
  },
  plugins: [],
};