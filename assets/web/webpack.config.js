const webpack = require('webpack');
const path = require('path');
const CopyPlugin = require("copy-webpack-plugin");
const HtmlMinimizerPlugin = require("html-minimizer-webpack-plugin");
const CssMinimizerPlugin = require('css-minimizer-webpack-plugin');
const MiniCssExtractPlugin = require("mini-css-extract-plugin");
const devMode = false; //process.env.NODE_ENV !== "production";

module.exports = {
	entry: {
		index: ['./src/index.js', './src/index.css'],
	},
	mode: devMode ? "development" : "production",
	devtool: "source-map", // Always generate source maps
	module: {
		rules: [
			{
				test: /\.css$/i,
				use: [
					devMode ? { loader: "style-loader" } : { loader: MiniCssExtractPlugin.loader, options: {} },
					{ loader: "css-loader" },
					{ loader: "postcss-loader", options: { postcssOptions: {
						plugins: [
							require('postcss-preset-env')({ stage: 2 }),
						],
						minimize: !devMode,
					}}},
				],
			},
		],
	},
	optimization: {
		minimize:  !devMode,
		minimizer: [
			`...`,
			new CssMinimizerPlugin(),
			new HtmlMinimizerPlugin(),
		]
	},
	plugins: [
		new webpack.NoEmitOnErrorsPlugin(),
		new CopyPlugin({
			patterns: [
				{
					context: path.resolve(__dirname, "src"),
					from: "./**/*.html",
				},
			],
		}),
	].concat(devMode ? [] : [new MiniCssExtractPlugin()]),
};

