# Overbot

## Event Processing and handling bot

This bot is built to take a set of `Event`s injected from a set of sources, to be
processed by a set of pre-check filters, to then be passed and handled by sinks.

A common example would be an IRC and Discord connections to be both Sources and Sinks
that can pass messages between each other, or a command processor that can respond to
a set of commands.
