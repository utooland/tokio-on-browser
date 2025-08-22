const path = require("path");
const HtmlWebpackPlugin = require("html-webpack-plugin");

module.exports = [
  {
    mode: "development",
    entry: {
      main: {
        import: "./js/index.js",
        filename: "main.js",
      }
    },
    devtool: "eval-source-map",
    output: {
      path: path.resolve(__dirname, "dist"),
    },
    plugins: [
      new HtmlWebpackPlugin({
        template: "./index.html",
        title: "Tokio on browser",
      }),
    ],
    devServer: {
      devMiddleware: {
        writeToDisk: true,
      },
      headers: {
        "Access-Control-Allow-Origin": "*",
        "Cross-Origin-Opener-Policy": "same-origin",
        "Cross-Origin-Embedder-Policy": "require-corp",
      },
      hot: false,
      liveReload: false,
      client: false,
      webSocketServer: false,
      port: 9091,
    },
  },
  {
    mode: "development",
    entry: {
      "tokio_worker": {
        import: "./js/tokio_worker.js",
        filename: "tokio_worker.js",
        chunkLoading: false
      },
    },
    devtool: "eval-source-map",
    output: {
      path: path.resolve(__dirname, "dist"),
    },
  }
];
