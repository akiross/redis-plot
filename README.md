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

 - One window for each plot type.
 - Multiple series for each window.
 - Using lists or streams as data structures.

API will be something like this

```
> rpush my_data 1 2 3 4 5
> rsp.scatter my_data
> rsp.lines my_data
```
