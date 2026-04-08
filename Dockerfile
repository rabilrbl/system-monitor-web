FROM node:22-alpine
WORKDIR /app
COPY system-monitor-server.js system-monitor.html ./
ENV PORT=8765
EXPOSE 8765
CMD ["node", "system-monitor-server.js"]
