// Local-only TCP proxy used inside isolated Docker network namespaces.
// It lets each browser see a secure-context http://127.0.0.1 origin while
// forwarding all HTTP/WebSocket requests to the same test server on the host.
const net = require('node:net');

const localPort = Number(process.env.LOCAL_ORIGIN_PORT || 8765);
const remotePort = Number(process.env.HOST_ORIGIN_PORT || 8765);
const remoteHost = process.env.HOST_ORIGIN || 'host.docker.internal';

const server = net.createServer(client => {
  const upstream = net.createConnection({ host: remoteHost, port: remotePort });
  client.setNoDelay(true);
  upstream.setNoDelay(true);
  client.pipe(upstream);
  upstream.pipe(client);
  const closeBoth = () => {
    client.destroy();
    upstream.destroy();
  };
  client.on('error', closeBoth);
  upstream.on('error', closeBoth);
});
server.on('error', error => { console.error(error); process.exitCode = 1; });
server.listen(localPort, '127.0.0.1', () => {
  console.log(`Namespace origin proxy listening on localhost:${localPort}`);
});