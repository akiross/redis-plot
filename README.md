# redis-plot module

This is a simple module that uses redis to collect data and plot them on screen.

For now it's simple and stupid: plots a single curve reading it from a list.

## Usage

```
$ cargo build
$ redis-server --loadmodule path/to/libredis_plot.so
```

then

```
$ redis-cli
> rpush rsp 1 4 9 16 25
> rsp.draw
```

## Future

The tool is not meant to become super duper complex, use gnuplot for that.

 - One plot kind/type for each window.
 - One or more series for each plot (window).
 - Using lists (or streams) as data structures.
 - Commands to plot in immediate mode or to automatically update the plot.
 - Output targets will be windows, files and binary strings.
   - A web client and/or jupyter integration is a cool idea, but not planned
     right now - with the same features OFC.

API will be something like this

```
# We need keyspace notification on lists (this could be done automatically)
> config set notify-keyspace-events Kl

# When my_data is updated, draw a scatter  plot
> rsp.scatter my_data

# When my_data is updated, draw a line plot
> rsp.lines my_data

# Finally data gets updated somehow, plot windows will change
> rpush my_data 1 2 3 4 5
```

### Command syntax

The commands shall support

 - different data sources (one or more lists, possibly streams)
 - different plot kinds (scatter, lines, bars, histograms)
 - different render targets (window, file, byte strings)
 - immediate/bind modes
 - plot options (colors, sizes, labels, legends...)

Given these premises, commands shall be designed in a way which is

1. clear to understand, using a simple, consistent scheme;
2. covenient to use, by not being too verbose;
3. easy on the memory, being "natural" (principle of least suprise);
4. adequate for redis, which has some bias on commands and parsing.

Possible command syntaxes, pros/cons:
 - gnuplot-like
   - pro: it exists already, might be easy to scrape
   - pro: users familiar with gnuplot are in advantage
   - con: familiar users might be surprised by missing features
   - con: not very clear and definitely not simple
   - con: not easy on memory
 - keyword based "command key1=val key2=val"
   - pro: conceptually simple, easy to remember
   - pro: flexible, order doesn't matter, very clear what is set
   - con: verbose?
   - con: role of spaces is unclear, might need decent parsing
 - keyword based with groups "command key1 val val key2 val val"
   - pro: simple, ice on redis parsing
   - pro: not too verbose, but is also explicit
   - pro: order doesn't matter
   - pro: flexible
   - con: unclear role of repeating keys `k1 v1 v2 k2 v3 k1 v4`
 - positional "command val1 val2 val3 ..."
   - pro: very simple, shell-like, nice on redis parsing
   - con: not very flexible, depends on position only
