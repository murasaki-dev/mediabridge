FROM node:24-bookworm-slim AS browser
ADD --checksum=sha256:11b53d27cd743e00e196e216920e7f3f794cf378e3f1792b8d25d3beabaf9cad https://github.com/h4ckf0r0day/obscura/releases/download/v0.2.7/obscura-x86_64-linux.tar.gz /tmp/obscura.tar.gz
RUN tar -xzf /tmp/obscura.tar.gz -C /usr/local/bin obscura

FROM node:24-bookworm-slim AS build
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY . .
RUN npm run check && npm test && npm run build

FROM node:24-bookworm-slim
WORKDIR /app
COPY --from=build /app/build ./build
COPY --from=browser /usr/local/bin/obscura /usr/local/bin/obscura
COPY serve.mjs preview-browser.mjs youtube-search.mjs youtube-live.mjs .
COPY obscura ./obscura
RUN mkdir /app/data && chown node:node /app/data
USER node
EXPOSE 8080
CMD ["node","serve.mjs"]
