pub type Chain do
  Empty
  Link(Int, Chain)
end

pub struct Meter do
  value :: Int
  active :: Bool
end

pub fn copy_chain(chain :: Chain) -> Chain do
  case chain do
    Empty -> Empty
    Link(head, tail) -> Link(head, copy_chain(tail))
  end
end

pub fn meter_value(meter :: Meter) -> Int do
  meter.value
end
