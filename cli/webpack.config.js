const path = require('path');
const webpack = require('webpack');

module.exports = {
  entry: './src/attest-ui.ts',
  devtool: 'source-map',
  module: {
    rules: [
      {
        test: /\.ts$/,
        use: ['ts-loader'],
        exclude: /node_modules|(tests|\.test\.ts$)/,
      },
      {
        test: /\.js$/,
        enforce: 'pre',
        use: ['source-map-loader'],
      },
    ],
  },
  resolve: {
    extensions: ['.ts', '.js'],
    fallback: {
      "process": require.resolve("process/browser"),
      "buffer": require.resolve("buffer/"),
    },
  },
  output: {
    filename: 'bundle.js',
    path: path.resolve(__dirname, 'dist'),
  },
  plugins: [
    new webpack.ProvidePlugin({
      Buffer: ['buffer', 'Buffer'],
    }),
  ],
};
