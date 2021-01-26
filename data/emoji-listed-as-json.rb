#!/usr/bin/env ruby
# encoding: utf-8

require 'oj'
require 'unicode/emoji'

class String
  def safe_key_format
    safe_key_format = self.dup
    safe_key_format.gsub!(" ", "_")
    safe_key_format.gsub!("&", "and")
    safe_key_format.downcase!
    safe_key_format
  end
end

emoji_list = {}
Unicode::Emoji.list.keys.each do |l|
    emoji_list[l.safe_key_format] = {}
    emoji_list[l.safe_key_format]['category'] = l
    Unicode::Emoji.list(l).keys.each do |subcategory|
        emoji_list[l.safe_key_format][subcategory] = {}
        emoji_list[l.safe_key_format][subcategory]['emojis'] = []
        Unicode::Emoji.list(l, subcategory).each do |emoji|
            emoji_list[l.safe_key_format][subcategory]['emojis'] << emoji
        end
    end
end

File.open('./emoji.json', 'w+') do |f|
    f << Oj.dump(emoji_list)
end
