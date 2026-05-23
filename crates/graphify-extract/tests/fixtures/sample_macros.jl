module Macros

using Base: @inline, @noinline
import Random

# Macro definitions
macro mytrace(expr)
    return :(println("trace: ", $(string(expr))); $(esc(expr)))
end

macro perf(expr)
    quote
        start = time()
        result = $(esc(expr))
        elapsed = time() - start
        println("elapsed: ", elapsed, "s")
        result
    end
end

# Type parameters and constraints
struct Wrapper{T<:Number}
    value::T
end

# Multiple dispatch
function process(x::Int)
    return x * 2
end

function process(x::String)
    return uppercase(x)
end

function process(x::Vector{T}) where T<:Number
    return sum(x)
end

# Inline / noinline annotations
@inline function fast_add(a, b)
    return a + b
end

@noinline function slow_div(a, b)
    return a / b
end

# Use macros
function demo()
    @mytrace 1 + 1
    @perf process(42)
end

export Wrapper, process, demo

end # module Macros
