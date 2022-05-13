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

## Testing

To perform integration testing it is necessary to build the cdylib somewhere and
start redis-server using that module: this is done automatically by the tests in
tests/integration.rs, but building will take some time to build both the library
and the tests.
During development, it might be useful to use the `REDIS_PLOT_TEST_TARGET_DIR`
to specify where the cdylib shall be built: if unset, a safe, random temporary
directory will be used for the build, but that variable can be set to force a
directory to use, resulting in faster subsequent builts - thus more useful if
needed to run the test several times.

Use `cargo insta test` to perform the acceptance tests, possibly reviewing the
outcome as instructed. Use `show_rle_snap` to inspect the snapshots.

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
   - pro: simple, nice on redis parsing
   - pro: not too verbose, but is also explicit
   - pro: order doesn't matter
   - pro: flexible
   - con: unclear role of repeating keys `k1 v1 v2 k2 v3 k1 v4`
 - positional "command val1 val2 val3 ..."
   - pro: very simple, shell-like, nice on redis parsing
   - con: not very flexible, depends on position only
